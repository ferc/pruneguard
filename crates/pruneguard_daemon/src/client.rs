use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::metadata::DaemonMetadata;
use crate::protocol::{DaemonRequest, DaemonResponse, DaemonStatusInfo, read_frame, write_frame};

/// Errors from the daemon client.
#[derive(Debug, thiserror::Error)]
pub enum DaemonClientError {
    #[error("failed to connect to daemon: {0}")]
    Connect(std::io::Error),
    #[error("failed to read daemon metadata: {0}")]
    Metadata(#[from] crate::metadata::MetadataError),
    #[error("protocol error: {0}")]
    Protocol(std::io::Error),
    #[error("invalid response from daemon: {0}")]
    InvalidResponse(String),
    #[error("daemon returned an error: {0}")]
    DaemonError(String),
    #[error("daemon not available")]
    NotAvailable,
    #[error("version mismatch: client={client}, daemon={daemon}")]
    VersionMismatch { client: String, daemon: String },
    #[error("failed to auto-start daemon: {0}")]
    AutoStart(String),
    #[error("timed out waiting for daemon to become ready")]
    StartupTimeout,
}

/// Client for communicating with a running pruneguard daemon.
pub struct DaemonClient {
    port: u16,
    token: String,
    pid: u32,
    version: String,
}

impl DaemonClient {
    /// Try to connect to an existing daemon for the given project root.
    ///
    /// Returns `Ok(None)` if no daemon is running.
    pub fn try_connect(project_root: &Path) -> Result<Option<Self>, DaemonClientError> {
        let metadata = DaemonMetadata::load(project_root)?;
        match metadata {
            Some(meta) => {
                // Verify version compatibility.
                let client_version = env!("CARGO_PKG_VERSION");
                if meta.version != client_version {
                    return Err(DaemonClientError::VersionMismatch {
                        client: client_version.to_string(),
                        daemon: meta.version,
                    });
                }
                Ok(Some(Self {
                    port: meta.port,
                    token: meta.token,
                    pid: meta.pid,
                    version: meta.version,
                }))
            }
            None => Ok(None),
        }
    }

    /// Connect to a running daemon, auto-starting one if needed.
    ///
    /// If no daemon is running, this spawns a new daemon process in the
    /// background and waits for it to become ready (up to ~5 seconds).
    pub fn connect_or_start(project_root: &Path) -> Result<Self, DaemonClientError> {
        // First, check if a daemon is already running.
        if let Some(client) = Self::try_connect(project_root)? {
            return Ok(client);
        }

        // No daemon running -- auto-start one.
        Self::auto_start(project_root)?;

        // Wait for the daemon to write its metadata file and become ready.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut poll_interval = Duration::from_millis(20);
        loop {
            if Instant::now() > deadline {
                return Err(DaemonClientError::StartupTimeout);
            }
            std::thread::sleep(poll_interval);
            // Increase poll interval geometrically: 20, 40, 80, 160, ...
            poll_interval = (poll_interval * 2).min(Duration::from_millis(500));

            if let Some(client) = Self::try_connect(project_root)? {
                return Ok(client);
            }
        }
    }

    /// Try to connect to a running, warm daemon. If no daemon is running,
    /// spawn one in the background for future runs but return `None` so the
    /// caller falls back to one-shot for this invocation.
    ///
    /// This is the preferred method for `--daemon auto`: the current command
    /// is not delayed by daemon startup, but subsequent commands will benefit
    /// from the warm daemon.
    pub fn try_connect_or_background_start(
        project_root: &Path,
    ) -> Result<Option<Self>, DaemonClientError> {
        if let Some(client) = Self::try_connect(project_root)? {
            Ok(Some(client))
        } else {
            // Spawn the daemon in the background. Ignore errors -- this is
            // best-effort.
            if let Err(err) = Self::auto_start(project_root) {
                tracing::debug!("background daemon start failed: {err}");
            }
            Ok(None)
        }
    }

    /// Spawn a daemon process in the background.
    fn auto_start(project_root: &Path) -> Result<(), DaemonClientError> {
        let exe = std::env::current_exe().map_err(|err| {
            DaemonClientError::AutoStart(format!("cannot find executable: {err}"))
        })?;

        let mut cmd = std::process::Command::new(exe);
        cmd.arg("daemon")
            .arg("start")
            .current_dir(project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // On Unix, start in a new process group so it survives the parent exiting.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        cmd.spawn().map_err(|err| DaemonClientError::AutoStart(format!("spawn failed: {err}")))?;

        tracing::info!("auto-started daemon for {}", project_root.display());
        Ok(())
    }

    /// Send a request to the daemon and return the response.
    pub fn send_request(
        &self,
        request: &DaemonRequest,
    ) -> Result<DaemonResponse, DaemonClientError> {
        let rt =
            tokio::runtime::Builder::new_current_thread().enable_io().build().map_err(|err| {
                DaemonClientError::Protocol(std::io::Error::other(format!(
                    "failed to create tokio runtime: {err}"
                )))
            })?;
        rt.block_on(self.send_request_async(request))
    }

    /// Send a request to the daemon asynchronously.
    async fn send_request_async(
        &self,
        request: &DaemonRequest,
    ) -> Result<DaemonResponse, DaemonClientError> {
        let stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", self.port))
            .await
            .map_err(DaemonClientError::Connect)?;

        let (mut reader, mut writer) = stream.into_split();

        // Send auth token as first frame.
        write_frame(&mut writer, self.token.as_bytes())
            .await
            .map_err(DaemonClientError::Protocol)?;

        // Send request.
        let payload = serde_json::to_vec(request).expect("serialize request");
        write_frame(&mut writer, &payload).await.map_err(DaemonClientError::Protocol)?;

        // Read response.
        let data = read_frame(&mut reader).await.map_err(DaemonClientError::Protocol)?.ok_or_else(
            || DaemonClientError::InvalidResponse("connection closed before response".to_string()),
        )?;

        let response: DaemonResponse = serde_json::from_slice(&data)
            .map_err(|err| DaemonClientError::InvalidResponse(format!("invalid JSON: {err}")))?;

        Ok(response)
    }

    /// Query the daemon's status.
    pub fn status(&self) -> Result<DaemonStatusInfo, DaemonClientError> {
        match self.send_request(&DaemonRequest::Status)? {
            DaemonResponse::Status { info } => Ok(info),
            DaemonResponse::Error { message } => Err(DaemonClientError::DaemonError(message)),
            other => Err(DaemonClientError::InvalidResponse(format!(
                "expected Status response, got {:?}",
                std::mem::discriminant(&other),
            ))),
        }
    }

    /// Port of the connected daemon.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// PID of the connected daemon.
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// Version of the connected daemon.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Returns `true` if the process appears to be running in a CI environment.
///
/// Checks the common `CI` env var used by GitHub Actions, GitLab CI, `CircleCI`,
/// Travis CI, Jenkins, and most other CI providers.
pub fn is_ci() -> bool {
    std::env::var("CI")
        .ok()
        .is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
}
