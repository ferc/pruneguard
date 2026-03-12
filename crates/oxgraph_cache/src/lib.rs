// Stub implementations — allow clippy lints that will resolve once redb is wired.
#![allow(clippy::unnecessary_wraps, clippy::unused_self, clippy::missing_const_for_fn)]

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Cache for incremental analysis using redb.
pub struct AnalysisCache {
    _db_path: PathBuf,
    // TODO: redb database handle
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("failed to open cache database: {0}")]
    OpenError(String),
    #[error("cache corrupted, will rebuild")]
    Corrupted,
}

impl AnalysisCache {
    /// Open or create the cache database.
    pub fn open(project_root: &Path) -> Result<Self, CacheError> {
        let db_path = project_root.join(".oxgraph-cache");
        // TODO: open redb
        Ok(Self { _db_path: db_path })
    }

    /// Check if a file's extracted facts are still valid.
    pub fn is_valid(&self, _path: &Path, _content_hash: u64) -> bool {
        // TODO: lookup in redb
        false
    }

    /// Invalidate and rebuild cache.
    pub fn clear(&self) {
        // TODO: clear redb tables
    }
}
