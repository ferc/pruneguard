//! Client for spawning and communicating with the `pruneguard-tsgo` semantic
//! helper binary.
//!
//! The client manages the helper subprocess lifecycle, sends query batches,
//! and collects results. It enforces timeouts and batch size limits.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pruneguard_semantic_protocol::{
    ErrorMessage, HEADER_SIZE, HandshakeRequest, MAX_PAYLOAD_SIZE, MessageType, PROTOCOL_VERSION,
    QueryBatch, ReadyMessage, ResponseBatch, decode_header, encode_message,
};
use thiserror::Error;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SemanticClientError {
    #[error("helper binary not found: {0}")]
    BinaryNotFound(String),

    #[error("helper failed to start: {0}")]
    SpawnFailed(#[from] std::io::Error),

    #[error("helper initialization timed out after {0}ms")]
    InitTimeout(u64),

    #[error("helper returned incompatible protocol version {got}, expected {expected}")]
    VersionMismatch { expected: u32, got: u32 },

    #[error("helper sent error: {0}")]
    HelperError(String),

    #[error("helper sent fatal error: {0}")]
    FatalError(String),

    #[error("helper query timed out after {0}ms")]
    QueryTimeout(u64),

    #[error("invalid message from helper: {0}")]
    ProtocolError(String),

    #[error("serialization error: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: u32, max: u32 },
}

// ---------------------------------------------------------------------------
// Client configuration
// ---------------------------------------------------------------------------

/// Configuration for the semantic client.
#[derive(Debug, Clone)]
pub struct SemanticClientConfig {
    /// Maximum wall-clock milliseconds for initialization.
    pub init_timeout_ms: u64,
    /// Maximum wall-clock milliseconds per query batch.
    pub query_timeout_ms: u64,
    /// Maximum files per query batch.
    pub max_files_per_batch: usize,
    /// Maximum TypeScript project references to load.
    pub max_project_refs: usize,
}

impl Default for SemanticClientConfig {
    fn default() -> Self {
        Self {
            init_timeout_ms: 5000,
            query_timeout_ms: 1200,
            max_files_per_batch: 128,
            max_project_refs: 8,
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// A client that manages the semantic helper subprocess.
pub struct SemanticClient {
    child: Child,
    config: SemanticClientConfig,
    ready: ReadyMessage,
    total_query_ms: u64,
    total_queries: usize,
}

/// Result of attempting to discover the helper binary.
pub enum HelperDiscovery {
    Found(PathBuf),
    NotFound(String),
}

impl SemanticClient {
    /// Discover the helper binary location.
    ///
    /// Search order:
    /// 1. `PRUNEGUARD_TSGO_PATH` environment variable
    /// 2. Walk up from `project_root` looking for `node_modules/.bin/pruneguard-tsgo`
    /// 3. Search system `PATH`
    pub fn discover_binary(project_root: &Path) -> HelperDiscovery {
        // 1. Environment variable
        if let Ok(path) = std::env::var("PRUNEGUARD_TSGO_PATH") {
            let path = PathBuf::from(&path);
            if path.exists() {
                return HelperDiscovery::Found(path);
            }
            return HelperDiscovery::NotFound(format!(
                "PRUNEGUARD_TSGO_PATH={} does not exist",
                path.display()
            ));
        }

        // 2. Walk up directory tree
        let bin_name = if cfg!(windows) { "pruneguard-tsgo.cmd" } else { "pruneguard-tsgo" };
        let mut dir = Some(project_root);
        while let Some(current) = dir {
            let candidate = current.join("node_modules/.bin").join(bin_name);
            if candidate.exists() {
                return HelperDiscovery::Found(candidate);
            }
            dir = current.parent();
        }

        // 3. System PATH
        if let Ok(output) = Command::new("which").arg("pruneguard-tsgo").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return HelperDiscovery::Found(PathBuf::from(path));
                }
            }
        }

        HelperDiscovery::NotFound(
            "pruneguard-tsgo not found in PRUNEGUARD_TSGO_PATH, node_modules, or PATH".to_string(),
        )
    }

    /// Spawn the helper and perform the handshake.
    pub fn spawn(
        binary_path: &Path,
        project_root: &str,
        tsconfig_paths: Vec<String>,
        config: SemanticClientConfig,
    ) -> Result<Self, SemanticClientError> {
        info!(binary = %binary_path.display(), "spawning semantic helper");

        let mut child = Command::new(binary_path)
            .arg("headless")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let handshake = HandshakeRequest {
            version: PROTOCOL_VERSION,
            project_root: project_root.to_string(),
            tsconfig_paths,
        };

        let payload = serde_json::to_vec(&handshake)?;
        let msg = encode_message(MessageType::Query, &payload);

        let stdin = child.stdin.as_mut().ok_or_else(|| {
            SemanticClientError::ProtocolError("failed to get stdin handle".to_string())
        })?;
        stdin.write_all(&msg)?;
        stdin.flush()?;

        // Wait for Ready message
        let started = Instant::now();
        let timeout = Duration::from_millis(config.init_timeout_ms);
        let stdout = child.stdout.as_mut().ok_or_else(|| {
            SemanticClientError::ProtocolError("failed to get stdout handle".to_string())
        })?;

        let (msg_type, payload) = read_message(stdout, Some(timeout - started.elapsed()))?;
        match msg_type {
            MessageType::Ready => {
                let ready: ReadyMessage = serde_json::from_slice(&payload)?;
                if ready.version != PROTOCOL_VERSION {
                    return Err(SemanticClientError::VersionMismatch {
                        expected: PROTOCOL_VERSION,
                        got: ready.version,
                    });
                }
                debug!(
                    projects = ready.projects_loaded,
                    files = ready.files_indexed,
                    init_ms = ready.init_ms,
                    "semantic helper ready"
                );
                Ok(Self { child, config, ready, total_query_ms: 0, total_queries: 0 })
            }
            MessageType::Error => {
                let err: ErrorMessage = serde_json::from_slice(&payload)?;
                if err.fatal {
                    Err(SemanticClientError::FatalError(err.error))
                } else {
                    Err(SemanticClientError::HelperError(err.error))
                }
            }
            _ => Err(SemanticClientError::ProtocolError(format!(
                "expected Ready or Error, got {:?}",
                msg_type
            ))),
        }
    }

    /// Send a query batch and receive results.
    pub fn query(&mut self, batch: QueryBatch) -> Result<ResponseBatch, SemanticClientError> {
        let payload = serde_json::to_vec(&batch)?;
        let msg = encode_message(MessageType::Query, &payload);

        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| SemanticClientError::ProtocolError("stdin closed".to_string()))?;
        stdin.write_all(&msg)?;
        stdin.flush()?;

        let timeout = Duration::from_millis(self.config.query_timeout_ms);
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| SemanticClientError::ProtocolError("stdout closed".to_string()))?;

        let (msg_type, payload) = read_message(stdout, Some(timeout))?;
        match msg_type {
            MessageType::Response => {
                let response: ResponseBatch = serde_json::from_slice(&payload)?;
                self.total_query_ms += response.batch_ms;
                self.total_queries += response.results.len();
                Ok(response)
            }
            MessageType::Error => {
                let err: ErrorMessage = serde_json::from_slice(&payload)?;
                if err.fatal {
                    Err(SemanticClientError::FatalError(err.error))
                } else {
                    Err(SemanticClientError::HelperError(err.error))
                }
            }
            _ => Err(SemanticClientError::ProtocolError(format!(
                "expected Response or Error, got {:?}",
                msg_type
            ))),
        }
    }

    /// Gracefully shut down the helper.
    pub fn shutdown(mut self) -> Result<(), SemanticClientError> {
        let msg = encode_message(MessageType::Shutdown, b"{}");
        if let Some(stdin) = self.child.stdin.as_mut() {
            let _ = stdin.write_all(&msg);
            let _ = stdin.flush();
        }
        let _ = self.child.wait();
        Ok(())
    }

    /// Get the ready message from initialization.
    pub fn ready_info(&self) -> &ReadyMessage {
        &self.ready
    }

    /// Total wall-clock milliseconds spent in queries.
    pub fn total_query_ms(&self) -> u64 {
        self.total_query_ms
    }

    /// Total number of individual queries sent.
    pub fn total_queries(&self) -> usize {
        self.total_queries
    }
}

impl Drop for SemanticClient {
    fn drop(&mut self) {
        // Best-effort: send shutdown and kill if needed.
        let msg = encode_message(MessageType::Shutdown, b"{}");
        if let Some(stdin) = self.child.stdin.as_mut() {
            let _ = stdin.write_all(&msg);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

fn read_message(
    reader: &mut impl Read,
    timeout: Option<Duration>,
) -> Result<(MessageType, Vec<u8>), SemanticClientError> {
    let _deadline = timeout.map(|t| Instant::now() + t);

    // Read header
    let mut header = [0u8; HEADER_SIZE];
    reader.read_exact(&mut header).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            SemanticClientError::ProtocolError("helper closed connection".to_string())
        } else {
            SemanticClientError::SpawnFailed(e)
        }
    })?;

    let (size, msg_type) = decode_header(&header).ok_or_else(|| {
        SemanticClientError::ProtocolError(format!("invalid header: {:?}", header))
    })?;

    if size > MAX_PAYLOAD_SIZE {
        return Err(SemanticClientError::PayloadTooLarge { size, max: MAX_PAYLOAD_SIZE });
    }

    // Read payload
    let mut payload = vec![0u8; size as usize];
    reader
        .read_exact(&mut payload)
        .map_err(|e| SemanticClientError::ProtocolError(format!("failed to read payload: {e}")))?;

    Ok((msg_type, payload))
}
