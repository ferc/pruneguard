use std::path::{Path, PathBuf};

use redb::{Database, TableDefinition};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const META_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const FILES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
const RESOLUTIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("resolutions");
const MANIFESTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("manifests");
const PATH_INDEX_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("path_index");

/// Cache for incremental analysis using redb.
pub struct AnalysisCache {
    db: Database,
    db_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedFileFacts {
    pub path: String,
    pub relative_path: String,
    pub file_hash: u64,
    pub config_hash: u64,
    pub resolver_hash: u64,
    pub manifest_hash: u64,
    pub tsconfig_hash: u64,
    pub facts_json: Vec<u8>,
    pub parse_diagnostics: Vec<String>,
    pub external_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedResolutions {
    pub path: String,
    pub resolved_imports_json: Vec<u8>,
    pub resolved_reexports_json: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedManifest {
    pub workspace: String,
    pub manifest_hash: u64,
    pub package_name: Option<String>,
    pub scripts_json: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathIndexEntry {
    pub relative_path: String,
    pub absolute_path: String,
    pub workspace: Option<String>,
    pub package: Option<String>,
    pub manifest_hash: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CacheCounters {
    pub hits: usize,
    pub misses: usize,
    pub entries_read: usize,
    pub entries_written: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("failed to open cache database: {0}")]
    OpenError(String),
    #[error("failed to read cache entry: {0}")]
    ReadError(String),
    #[error("failed to write cache entry: {0}")]
    WriteError(String),
    #[error("failed to serialize cache entry: {0}")]
    SerializeError(String),
    #[error("cache corrupted, will rebuild")]
    Corrupted,
}

impl AnalysisCache {
    /// Open or create the cache database.
    pub fn open(project_root: &Path) -> Result<Self, CacheError> {
        let cache_dir = project_root.join(".pruneguard");
        std::fs::create_dir_all(&cache_dir)
            .map_err(|err| CacheError::OpenError(err.to_string()))?;
        let db_path = cache_dir.join("cache.redb");
        let db =
            Database::create(&db_path).map_err(|err| CacheError::OpenError(err.to_string()))?;

        let write_txn = db.begin_write().map_err(|err| CacheError::OpenError(err.to_string()))?;
        write_txn.open_table(META_TABLE).map_err(|err| CacheError::OpenError(err.to_string()))?;
        write_txn.open_table(FILES_TABLE).map_err(|err| CacheError::OpenError(err.to_string()))?;
        write_txn
            .open_table(RESOLUTIONS_TABLE)
            .map_err(|err| CacheError::OpenError(err.to_string()))?;
        write_txn
            .open_table(MANIFESTS_TABLE)
            .map_err(|err| CacheError::OpenError(err.to_string()))?;
        write_txn
            .open_table(PATH_INDEX_TABLE)
            .map_err(|err| CacheError::OpenError(err.to_string()))?;
        write_txn.commit().map_err(|err| CacheError::OpenError(err.to_string()))?;

        Ok(Self { db, db_path })
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    pub fn get_file_facts(&self, path: &Path) -> Result<Option<CachedFileFacts>, CacheError> {
        self.get_json(FILES_TABLE, &path.to_string_lossy())
    }

    pub fn put_file_facts(&self, entry: &CachedFileFacts) -> Result<(), CacheError> {
        self.put_json(FILES_TABLE, &entry.path, entry)
    }

    pub fn get_resolutions(&self, path: &Path) -> Result<Option<CachedResolutions>, CacheError> {
        self.get_json(RESOLUTIONS_TABLE, &path.to_string_lossy())
    }

    pub fn put_resolutions(&self, entry: &CachedResolutions) -> Result<(), CacheError> {
        self.put_json(RESOLUTIONS_TABLE, &entry.path, entry)
    }

    pub fn put_manifest(&self, entry: &CachedManifest) -> Result<(), CacheError> {
        self.put_json(MANIFESTS_TABLE, &entry.workspace, entry)
    }

    pub fn lookup_manifest(&self, workspace: &str) -> Result<Option<CachedManifest>, CacheError> {
        self.get_json(MANIFESTS_TABLE, workspace)
    }

    pub fn record_path_index(&self, entry: &PathIndexEntry) -> Result<(), CacheError> {
        self.put_json(PATH_INDEX_TABLE, &entry.relative_path, entry)
    }

    pub fn lookup_path_index(
        &self,
        relative_path: &Path,
    ) -> Result<Option<PathIndexEntry>, CacheError> {
        self.get_json(PATH_INDEX_TABLE, &relative_path.to_string_lossy())
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), CacheError> {
        self.put_json(META_TABLE, key, &value.to_string())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>, CacheError> {
        self.get_json(META_TABLE, key)
    }

    /// Invalidate and rebuild cache.
    pub fn clear(&self) -> Result<(), CacheError> {
        let write_txn =
            self.db.begin_write().map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .delete_table(FILES_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .delete_table(RESOLUTIONS_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .delete_table(MANIFESTS_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .delete_table(PATH_INDEX_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .delete_table(META_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn.commit().map_err(|err| CacheError::WriteError(err.to_string()))?;

        let write_txn =
            self.db.begin_write().map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn.open_table(META_TABLE).map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn.open_table(FILES_TABLE).map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .open_table(RESOLUTIONS_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .open_table(MANIFESTS_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn
            .open_table(PATH_INDEX_TABLE)
            .map_err(|err| CacheError::WriteError(err.to_string()))?;
        write_txn.commit().map_err(|err| CacheError::WriteError(err.to_string()))
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        table_def: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Result<Option<T>, CacheError> {
        let read_txn =
            self.db.begin_read().map_err(|err| CacheError::ReadError(err.to_string()))?;
        let table =
            read_txn.open_table(table_def).map_err(|err| CacheError::ReadError(err.to_string()))?;
        let Some(value) = table.get(key).map_err(|err| CacheError::ReadError(err.to_string()))?
        else {
            return Ok(None);
        };
        serde_json::from_slice(value.value())
            .map(Some)
            .map_err(|err| CacheError::SerializeError(err.to_string()))
    }

    fn put_json<T: Serialize>(
        &self,
        table_def: TableDefinition<&str, &[u8]>,
        key: &str,
        value: &T,
    ) -> Result<(), CacheError> {
        let bytes =
            serde_json::to_vec(value).map_err(|err| CacheError::SerializeError(err.to_string()))?;
        let write_txn =
            self.db.begin_write().map_err(|err| CacheError::WriteError(err.to_string()))?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .map_err(|err| CacheError::WriteError(err.to_string()))?;
            table
                .insert(key, bytes.as_slice())
                .map_err(|err| CacheError::WriteError(err.to_string()))?;
        }
        write_txn.commit().map_err(|err| CacheError::WriteError(err.to_string()))
    }
}
