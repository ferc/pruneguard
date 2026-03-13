use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Information written to `.pruneguard/daemon.json` while the daemon is running.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonMetadata {
    /// Process ID of the running daemon.
    pub pid: u32,
    /// TCP port the daemon is listening on.
    pub port: u16,
    /// Authentication token (random hex string).
    pub token: String,
    /// Absolute path to the project root.
    pub project_root: String,
    /// Version of pruneguard running the daemon.
    pub version: String,
    /// ISO-8601 timestamp when the daemon started.
    pub started_at: String,
}

/// Errors that can occur when managing daemon metadata.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("failed to read daemon metadata: {0}")]
    Read(std::io::Error),
    #[error("failed to write daemon metadata: {0}")]
    Write(std::io::Error),
    #[error("failed to parse daemon metadata: {0}")]
    Parse(serde_json::Error),
    #[error("failed to remove daemon metadata: {0}")]
    Remove(std::io::Error),
}

impl DaemonMetadata {
    /// Path to the metadata file for a given project root.
    pub fn path(project_root: &Path) -> PathBuf {
        project_root.join(".pruneguard").join("daemon.json")
    }

    /// Write daemon metadata to disk.
    pub fn write(&self, project_root: &Path) -> Result<(), MetadataError> {
        let dir = project_root.join(".pruneguard");
        std::fs::create_dir_all(&dir).map_err(MetadataError::Write)?;
        let path = Self::path(project_root);
        let json = serde_json::to_string_pretty(self).expect("serialize daemon metadata");
        std::fs::write(&path, json).map_err(MetadataError::Write)?;
        tracing::debug!("wrote daemon metadata to {}", path.display());
        Ok(())
    }

    /// Load and validate daemon metadata from disk.
    ///
    /// Returns `Ok(None)` if the file does not exist or contains stale information.
    pub fn load(project_root: &Path) -> Result<Option<Self>, MetadataError> {
        let path = Self::path(project_root);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path).map_err(MetadataError::Read)?;
        let meta: Self = serde_json::from_str(&contents).map_err(MetadataError::Parse)?;

        // Check if the process is still alive
        if !is_pid_alive(meta.pid) {
            tracing::debug!(
                "daemon metadata at {} references stale pid {}; removing",
                path.display(),
                meta.pid,
            );
            Self::cleanup(project_root).ok();
            return Ok(None);
        }

        Ok(Some(meta))
    }

    /// Remove the metadata file on shutdown.
    pub fn cleanup(project_root: &Path) -> Result<(), MetadataError> {
        let path = Self::path(project_root);
        if path.exists() {
            std::fs::remove_file(&path).map_err(MetadataError::Remove)?;
            tracing::debug!("removed daemon metadata at {}", path.display());
        }
        Ok(())
    }

    /// Generate a random hex token suitable for authentication.
    pub fn generate_token() -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        use std::time::SystemTime;

        let mut hasher = DefaultHasher::new();
        hasher.write_u128(
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos(),
        );
        hasher.write_u32(std::process::id());
        let h1 = hasher.finish();
        hasher.write_u64(h1.wrapping_mul(0x517c_c1b7_2722_0a95));
        let h2 = hasher.finish();
        format!("{h1:016x}{h2:016x}")
    }
}

/// Check whether a process with the given PID is alive.
fn is_pid_alive(pid: u32) -> bool {
    // On Unix, sending signal 0 checks for existence without affecting the process.
    #[cfg(unix)]
    {
        check_pid_unix(pid)
    }
    #[cfg(not(unix))]
    {
        // On non-Unix platforms, assume alive if we can't check.
        let _ = pid;
        true
    }
}

#[cfg(unix)]
fn check_pid_unix(pid: u32) -> bool {
    // `kill -0 <pid>` checks for process existence without sending a signal.
    // We avoid pulling in the `libc` crate by using `std::process::Command`.
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
