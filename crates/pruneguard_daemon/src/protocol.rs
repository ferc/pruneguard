use serde::{Deserialize, Serialize};

/// A request from a client to the daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DaemonRequest {
    /// Run a full scan, optionally scoped to specific paths.
    Scan {
        /// Paths to analyze (empty means project root).
        #[serde(default)]
        paths: Vec<String>,
        /// Only analyze files changed since this git ref.
        #[serde(default)]
        changed_since: Option<String>,
        /// Focus analysis on files matching this glob.
        #[serde(default)]
        focus: Option<String>,
    },
    /// Run a review (CI gate check) against the current branch.
    Review {
        /// Base ref for changed-file detection.
        #[serde(default)]
        base_ref: Option<String>,
    },
    /// Compute the blast radius for a target file or export.
    Impact {
        /// The file or export to analyze.
        target: String,
        /// Focus on files matching this glob.
        #[serde(default)]
        focus: Option<String>,
    },
    /// Explain a finding or path.
    Explain {
        /// Finding ID or file path to explain.
        query: String,
        /// Focus on files matching this glob.
        #[serde(default)]
        focus: Option<String>,
    },
    /// Evaluate targets for safe deletion.
    SafeDelete {
        /// Files or exports to evaluate.
        targets: Vec<String>,
    },
    /// Generate a fix plan for the given targets.
    FixPlan {
        /// Files or exports to generate a fix plan for.
        targets: Vec<String>,
    },
    /// Query the daemon's status.
    Status,
    /// Request a graceful shutdown.
    Shutdown,
}

/// A response from the daemon to a client.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DaemonResponse {
    /// Result of a scan request.
    ScanResult {
        /// The full scan report as JSON.
        report: serde_json::Value,
    },
    /// Result of a review request.
    ReviewResult {
        /// The review report as JSON.
        report: serde_json::Value,
    },
    /// Result of an impact request.
    ImpactResult {
        /// The impact report as JSON.
        report: serde_json::Value,
    },
    /// Result of an explain request.
    ExplainResult {
        /// The explain report as JSON.
        report: serde_json::Value,
    },
    /// Result of a safe-delete request.
    SafeDeleteResult {
        /// The safe-delete report as JSON.
        report: serde_json::Value,
    },
    /// Result of a fix-plan request.
    FixPlanResult {
        /// The fix-plan report as JSON.
        report: serde_json::Value,
    },
    /// Daemon status information.
    Status {
        /// Detailed status info.
        info: DaemonStatusInfo,
    },
    /// Generic success acknowledgment (e.g. for shutdown).
    Ok,
    /// An error occurred processing the request.
    Error {
        /// Human-readable error message.
        message: String,
    },
}

/// Status information returned by the daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatusInfo {
    /// Absolute path to the project root being watched.
    pub project_root: String,
    /// Whether the daemon is running.
    pub running: bool,
    /// Process ID of the daemon.
    pub pid: u32,
    /// Version of pruneguard running the daemon.
    pub version: String,
    /// TCP port the daemon is listening on.
    pub port: u16,
    /// Whether the hot index has been warmed (initial build complete).
    pub index_warm: bool,
    /// Milliseconds since last graph update.
    pub last_update_ms: u64,
    /// Number of files being watched for changes.
    pub watched_files: usize,
    /// Number of nodes in the module graph.
    pub graph_nodes: usize,
    /// Number of edges in the module graph.
    pub graph_edges: usize,
}

/// Read a length-prefixed JSON frame from a tokio `AsyncRead`.
///
/// Wire format: 4-byte big-endian length prefix followed by JSON payload.
pub async fn read_frame<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    let mut len_buf = [0u8; 4];
    match tokio::io::AsyncReadExt::read_exact(reader, &mut len_buf).await {
        Ok(_n) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes"),
        ));
    }
    let mut buf = vec![0u8; len];
    tokio::io::AsyncReadExt::read_exact(reader, &mut buf).await?;
    Ok(Some(buf))
}

/// Write a length-prefixed JSON frame to a tokio `AsyncWrite`.
pub async fn write_frame<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), std::io::Error> {
    let len = u32::try_from(data.len()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "frame payload exceeds u32::MAX")
    })?;
    tokio::io::AsyncWriteExt::write_all(writer, &len.to_be_bytes()).await?;
    tokio::io::AsyncWriteExt::write_all(writer, data).await?;
    tokio::io::AsyncWriteExt::flush(writer).await?;
    Ok(())
}
