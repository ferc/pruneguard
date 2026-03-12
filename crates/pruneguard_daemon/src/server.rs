use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tokio::net::TcpListener;
use tokio::sync::{Mutex, watch};

use crate::index::HotIndex;
use crate::metadata::DaemonMetadata;
use crate::protocol::{
    DaemonRequest, DaemonResponse, DaemonStatusInfo, read_frame, write_frame,
};
use crate::watcher::FileWatcher;

/// Errors that can occur in the daemon server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("failed to bind TCP listener: {0}")]
    Bind(std::io::Error),
    #[error("failed to start file watcher: {0}")]
    Watcher(#[from] crate::watcher::WatcherError),
    #[error("failed to write daemon metadata: {0}")]
    Metadata(#[from] crate::metadata::MetadataError),
    #[error("index error: {0}")]
    Index(#[from] crate::index::IndexError),
}

/// The pruneguard daemon server.
///
/// Binds to a random loopback port, watches the file system for changes,
/// and serves analysis requests over a length-prefixed JSON protocol.
pub struct DaemonServer {
    project_root: PathBuf,
    index: Arc<Mutex<HotIndex>>,
    token: String,
    started_at: Instant,
}

impl DaemonServer {
    /// Create a new daemon server for the given project root and config.
    pub fn new(
        project_root: PathBuf,
        config: pruneguard_config::PruneguardConfig,
    ) -> Self {
        let token = DaemonMetadata::generate_token();
        Self {
            index: Arc::new(Mutex::new(HotIndex::new(project_root.clone(), config))),
            project_root,
            token,
            started_at: Instant::now(),
        }
    }

    /// Run the daemon: bind, warm the index, watch files, and serve requests.
    ///
    /// This blocks until the daemon is shut down.
    pub async fn run(self) -> Result<(), ServerError> {
        // Bind to a random available port on loopback.
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(ServerError::Bind)?;
        let addr = listener.local_addr().map_err(ServerError::Bind)?;
        let port = addr.port();
        tracing::info!("daemon listening on 127.0.0.1:{port}");

        // Write metadata file.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let started_at_iso = format_epoch_as_iso(now.as_secs());

        let metadata = DaemonMetadata {
            pid: std::process::id(),
            port,
            token: self.token.clone(),
            project_root: self.project_root.display().to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: started_at_iso,
        };
        metadata.write(&self.project_root)?;

        // Warm the index in a blocking task.
        {
            let index = Arc::clone(&self.index);
            tokio::task::spawn_blocking(move || {
                let mut idx = index.blocking_lock();
                if let Err(err) = idx.build_initial() {
                    tracing::error!("initial index build failed: {err}");
                }
            })
            .await
            .ok();
        }

        // Start file watcher.
        let mut watcher = FileWatcher::start(&self.project_root)?;

        // Shutdown signal channel.
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        // Spawn the file-change handler.
        let watcher_index = Arc::clone(&self.index);
        let watcher_handle = tokio::spawn(async move {
            while let Some(changed_paths) = watcher.changes_rx.recv().await {
                tracing::debug!("file changes detected: {} paths", changed_paths.len());
                let idx = Arc::clone(&watcher_index);
                tokio::task::spawn_blocking(move || {
                    let mut index = idx.blocking_lock();
                    index.invalidate_files(&changed_paths);
                    if let Err(err) = index.rebuild_changed() {
                        tracing::error!("rebuild after file changes failed: {err}");
                    }
                })
                .await
                .ok();
            }
        });

        // Accept connections.
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer)) => {
                            tracing::debug!("accepted connection from {peer}");
                            let index = Arc::clone(&self.index);
                            let token = self.token.clone();
                            let shutdown = shutdown_tx.clone();
                            let project_root = self.project_root.clone();
                            let started_at = self.started_at;
                            tokio::spawn(async move {
                                if let Err(err) = handle_connection(
                                    stream, &token, index, shutdown, &project_root, started_at,
                                ).await {
                                    tracing::debug!("connection handler error: {err}");
                                }
                            });
                        }
                        Err(err) => {
                            tracing::warn!("accept error: {err}");
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("daemon shutdown requested");
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("received SIGINT, shutting down daemon");
                    break;
                }
            }
        }

        // Cleanup.
        watcher_handle.abort();
        DaemonMetadata::cleanup(&self.project_root).ok();
        tracing::info!("daemon shut down cleanly");
        Ok(())
    }
}

/// Handle a single client connection.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    expected_token: &str,
    index: Arc<Mutex<HotIndex>>,
    shutdown_tx: watch::Sender<bool>,
    project_root: &Path,
    started_at: Instant,
) -> Result<(), std::io::Error> {
    let (mut reader, mut writer) = stream.split();

    // First frame must be the auth token.
    let auth_frame = read_frame(&mut reader).await?;
    match auth_frame {
        Some(data) => {
            let token = String::from_utf8_lossy(&data);
            if token.trim() != expected_token {
                let resp = DaemonResponse::Error {
                    message: "authentication failed".to_string(),
                };
                let json = serde_json::to_vec(&resp).unwrap_or_default();
                write_frame(&mut writer, &json).await?;
                return Ok(());
            }
        }
        None => return Ok(()),
    }

    // Process request frames.
    while let Some(data) = read_frame(&mut reader).await? {
        let request: DaemonRequest = match serde_json::from_slice(&data) {
            Ok(req) => req,
            Err(err) => {
                let resp = DaemonResponse::Error {
                    message: format!("invalid request: {err}"),
                };
                let json = serde_json::to_vec(&resp).unwrap_or_default();
                write_frame(&mut writer, &json).await?;
                continue;
            }
        };

        let response = dispatch_request(
            request,
            &index,
            &shutdown_tx,
            project_root,
            started_at,
        )
        .await;

        let json = serde_json::to_vec(&response).unwrap_or_default();
        write_frame(&mut writer, &json).await?;

        // If shutdown was requested, break after sending the OK response.
        if *shutdown_tx.borrow() {
            break;
        }
    }

    Ok(())
}

/// Dispatch a single request to the appropriate handler.
async fn dispatch_request(
    request: DaemonRequest,
    index: &Arc<Mutex<HotIndex>>,
    shutdown_tx: &watch::Sender<bool>,
    project_root: &Path,
    started_at: Instant,
) -> DaemonResponse {
    match request {
        DaemonRequest::Scan { paths, changed_since, focus } => {
            let idx = index.lock().await;
            let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
            match idx.query_scan(&path_bufs, changed_since.as_deref(), focus.as_deref()) {
                Ok(report) => DaemonResponse::ScanResult { report },
                Err(err) => DaemonResponse::Error { message: err.to_string() },
            }
        }
        DaemonRequest::Review { base_ref } => {
            let idx = index.lock().await;
            match idx.query_review(base_ref.as_deref()) {
                Ok(report) => DaemonResponse::ReviewResult { report },
                Err(err) => DaemonResponse::Error { message: err.to_string() },
            }
        }
        DaemonRequest::Impact { target, focus } => {
            let idx = index.lock().await;
            match idx.query_impact(&target, focus.as_deref()) {
                Ok(report) => DaemonResponse::ImpactResult { report },
                Err(err) => DaemonResponse::Error { message: err.to_string() },
            }
        }
        DaemonRequest::Explain { query, focus } => {
            let idx = index.lock().await;
            match idx.query_explain(&query, focus.as_deref()) {
                Ok(report) => DaemonResponse::ExplainResult { report },
                Err(err) => DaemonResponse::Error { message: err.to_string() },
            }
        }
        DaemonRequest::SafeDelete { targets } => {
            let idx = index.lock().await;
            match idx.query_safe_delete(&targets) {
                Ok(report) => DaemonResponse::SafeDeleteResult { report },
                Err(err) => DaemonResponse::Error { message: err.to_string() },
            }
        }
        DaemonRequest::FixPlan { targets } => {
            let idx = index.lock().await;
            match idx.query_fix_plan(&targets) {
                Ok(report) => DaemonResponse::FixPlanResult { report },
                Err(err) => DaemonResponse::Error { message: err.to_string() },
            }
        }
        DaemonRequest::Status => {
            let idx = index.lock().await;
            let uptime_secs = started_at.elapsed().as_secs();
            let info = DaemonStatusInfo {
                project_root: project_root.display().to_string(),
                running: true,
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                port: 0, // The caller already knows the port.
                index_warm: idx.is_warm(),
                last_update_ms: idx.last_update_ms(),
                watched_files: idx.tracked_files(),
                graph_nodes: idx.graph_nodes(),
                graph_edges: idx.graph_edges(),
                execution_mode: "daemon".to_string(),
                generation: idx.generation(),
                watcher_lag_ms: idx.watcher_lag_ms(),
                pending_invalidations: idx.pending_invalidations(),
                uptime_secs,
            };
            DaemonResponse::Status { info }
        }
        DaemonRequest::Shutdown => {
            tracing::info!("shutdown requested by client");
            shutdown_tx.send(true).ok();
            DaemonResponse::Ok
        }
    }
}

/// Format a Unix epoch timestamp as a simplified ISO-8601 string.
fn format_epoch_as_iso(epoch_secs: u64) -> String {
    // Simple UTC formatting without pulling in `chrono`.
    let secs_per_minute = 60u64;
    let secs_per_hour = 3600u64;
    let secs_per_day = 86400u64;

    let days = epoch_secs / secs_per_day;
    let remaining = epoch_secs % secs_per_day;
    let hours = remaining / secs_per_hour;
    let remaining = remaining % secs_per_hour;
    let minutes = remaining / secs_per_minute;
    let seconds = remaining % secs_per_minute;

    // Compute year/month/day from days since 1970-01-01 using a civil calendar algorithm.
    let (year, month, day) = days_to_civil(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since 1970-01-01 to (year, month, day).
///
/// Based on Howard Hinnant's `civil_from_days` algorithm.
const fn days_to_civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
