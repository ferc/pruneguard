use compact_str::CompactString;
use rustc_hash::FxHashMap;

use crate::ids::{ExportId, FileId};

/// The symbol-level graph tracking export/import liveness.
#[derive(Debug, Default, Clone)]
pub struct SymbolGraph {
    /// All exports, keyed by file + export name.
    pub exports: FxHashMap<(FileId, CompactString), ExportNode>,
    /// Reverse map: which files import a given export.
    pub import_edges: Vec<ImportEdge>,
    /// Re-export chains.
    pub reexport_edges: Vec<ReexportEdge>,
    /// Next export ID counter.
    next_export_id: u32,
}

/// An export node in the symbol graph.
#[derive(Debug, Clone)]
pub struct ExportNode {
    pub id: ExportId,
    pub file: FileId,
    pub name: CompactString,
    pub is_type: bool,
    /// Whether this export is live (reachable from an entrypoint).
    pub is_live: bool,
}

/// An edge representing an import of a specific export.
#[derive(Debug, Clone)]
pub struct ImportEdge {
    /// The file doing the importing.
    pub importer: FileId,
    /// The file being imported from.
    pub source: FileId,
    /// The export name being imported.
    pub export_name: CompactString,
    /// Whether this is a type-only import.
    pub is_type: bool,
}

/// An edge representing a re-export chain.
#[derive(Debug, Clone)]
pub struct ReexportEdge {
    /// The file doing the re-exporting.
    pub reexporter: FileId,
    /// The source file.
    pub source: FileId,
    /// Original name in the source.
    pub original_name: CompactString,
    /// Name as re-exported.
    pub exported_name: CompactString,
    /// Whether this is `export *`.
    pub is_star: bool,
    /// Whether this is a type-only re-export.
    pub is_type: bool,
}

impl SymbolGraph {
    /// Register an export.
    pub fn add_export(&mut self, file: FileId, name: CompactString, is_type: bool) -> ExportId {
        let id = ExportId(self.next_export_id);
        self.next_export_id += 1;
        self.exports
            .insert((file, name.clone()), ExportNode { id, file, name, is_type, is_live: false });
        id
    }

    /// Record an import edge.
    pub fn add_import(
        &mut self,
        importer: FileId,
        source: FileId,
        export_name: CompactString,
        is_type: bool,
    ) {
        self.import_edges.push(ImportEdge { importer, source, export_name, is_type });
    }

    /// Record a re-export edge.
    pub fn add_reexport(
        &mut self,
        reexporter: FileId,
        source: FileId,
        original_name: CompactString,
        exported_name: CompactString,
        is_star: bool,
        is_type: bool,
    ) {
        self.reexport_edges.push(ReexportEdge {
            reexporter,
            source,
            original_name,
            exported_name,
            is_star,
            is_type,
        });
    }

    /// Mark an export as live.
    pub fn mark_live(&mut self, file: FileId, name: &str) {
        if let Some(node) = self.exports.get_mut(&(file, CompactString::new(name))) {
            node.is_live = true;
        }
    }

    /// Mark all exports in a file as live.
    pub fn mark_all_file_exports_live(&mut self, file: FileId, is_type: Option<bool>) {
        for ((export_file, _), node) in &mut self.exports {
            if *export_file == file && is_type.is_none_or(|kind| kind == node.is_type) {
                node.is_live = true;
            }
        }
    }

    /// Return all exports for a specific file.
    pub fn exports_for_file(&self, file: FileId) -> impl Iterator<Item = &ExportNode> {
        self.exports.values().filter(move |node| node.file == file)
    }

    /// Get all dead (unused) exports.
    pub fn dead_exports(&self) -> impl Iterator<Item = &ExportNode> {
        self.exports.values().filter(|node| !node.is_live)
    }
}
