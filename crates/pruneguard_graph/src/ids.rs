use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Interned path ID for deduplication and fast comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub u32);

/// Interned workspace ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub u32);

/// Interned package ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId(pub u32);

/// Interned export ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExportId(pub u32);

/// Stable string ID for a graph node.
///
/// Format:
/// - `file:<workspace>:<relative-path>`
/// - `pkg:<workspace>:<package-name>`
/// - `export:<workspace>:<relative-path>#<export-name>`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StableId(pub CompactString);

impl StableId {
    pub fn file(workspace: &str, path: &str) -> Self {
        Self(CompactString::new(format!("file:{workspace}:{path}")))
    }

    pub fn package(workspace: &str, name: &str) -> Self {
        Self(CompactString::new(format!("pkg:{workspace}:{name}")))
    }

    pub fn export(workspace: &str, path: &str, export_name: &str) -> Self {
        Self(CompactString::new(format!("export:{workspace}:{path}#{export_name}")))
    }
}

impl std::fmt::Display for StableId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// String interner for deduplicating paths and names.
#[derive(Debug, Default)]
pub struct Interner {
    strings: Vec<CompactString>,
    map: rustc_hash::FxHashMap<CompactString, u32>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = u32::try_from(self.strings.len()).expect("interner exceeded u32::MAX entries");
        let cs = CompactString::new(s);
        self.map.insert(cs.clone(), id);
        self.strings.push(cs);
        id
    }

    pub fn resolve(&self, id: u32) -> &str {
        &self.strings[id as usize]
    }

    /// Look up the ID for a string, if it has been interned.
    pub fn lookup(&self, s: &str) -> Option<u32> {
        self.map.get(s).copied()
    }
}
