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
    /// Member access references (e.g. `MyEnum.Variant`).
    pub member_refs: Vec<MemberRef>,
    /// Same-file references to exports (not via import).
    pub same_file_refs: Vec<SameFileRef>,
    /// Individual members of exported classes/enums/namespaces.
    pub member_exports: Vec<MemberExportNode>,
    /// Namespace alias chains (destructured namespace members).
    pub namespace_alias_chains: Vec<NamespaceAliasChain>,
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

/// Tracks when a consumer accesses a specific member of an exported binding
/// (e.g. `MyEnum.Variant`, `instance.method`).
#[derive(Debug, Clone)]
pub struct MemberRef {
    /// File doing the access.
    pub accessor: FileId,
    /// File that exports the parent.
    pub source: FileId,
    /// Parent export name.
    pub export_name: CompactString,
    /// Member being accessed (e.g. method name, enum variant).
    pub member_name: CompactString,
    /// Whether this is a type-only access.
    pub is_type: bool,
    /// Whether this is a read access, write access, or both.
    pub access_kind: MemberAccessKind,
}

/// The kind of access to a member (read, write, or both).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MemberAccessKind {
    #[default]
    Read,
    Write,
    ReadWrite,
}

/// Tracks when an export is referenced within its own file (not via import).
#[derive(Debug, Clone)]
pub struct SameFileRef {
    /// File containing the export and the reference.
    pub file: FileId,
    /// The export name being referenced.
    pub export_name: CompactString,
    /// Line number of the reference.
    pub ref_line: u32,
}

/// Tracks individual members of exported classes/enums/namespaces.
#[derive(Debug, Clone)]
pub struct MemberExportNode {
    /// The parent export name.
    pub parent_export: CompactString,
    /// The member name.
    pub member_name: CompactString,
    /// File containing the parent export.
    pub file: FileId,
    /// Whether this member is live (reachable).
    pub is_live: bool,
    /// The kind of member (method, property, getter, setter, enum variant, etc.).
    pub member_kind: MemberNodeKind,
    /// Whether this member has a @public `JSDoc` tag.
    pub is_public_tagged: bool,
}

/// The kind of a member within an exported class, enum, or namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemberNodeKind {
    #[default]
    Property,
    Method,
    Getter,
    Setter,
    EnumVariant,
    StaticProperty,
    StaticMethod,
    NamespaceMember,
}

/// Tracks when a namespace import is destructured or aliased.
#[derive(Debug, Clone)]
pub struct NamespaceAliasChain {
    /// File containing the destructuring/alias.
    pub file: FileId,
    /// Source file of the namespace import.
    pub namespace_source: FileId,
    /// The namespace export name (often the module's own name or `*`).
    pub namespace_export: CompactString,
    /// Local binding name after destructuring/alias.
    pub local_name: CompactString,
    /// Member name from the namespace.
    pub member_name: CompactString,
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

    /// Record a member access (e.g. `MyEnum.Variant`, `instance.method`).
    pub fn add_member_ref(
        &mut self,
        accessor: FileId,
        source: FileId,
        export_name: CompactString,
        member_name: CompactString,
        is_type: bool,
        access_kind: MemberAccessKind,
    ) {
        self.member_refs.push(MemberRef {
            accessor,
            source,
            export_name,
            member_name,
            is_type,
            access_kind,
        });
    }

    /// Record a same-file reference to an export.
    pub fn add_same_file_ref(&mut self, file: FileId, export_name: CompactString, ref_line: u32) {
        self.same_file_refs.push(SameFileRef { file, export_name, ref_line });
    }

    /// Register a member of an exported class/enum/namespace.
    pub fn add_member_export(
        &mut self,
        file: FileId,
        parent_export: CompactString,
        member_name: CompactString,
        member_kind: MemberNodeKind,
        is_public_tagged: bool,
    ) {
        self.member_exports.push(MemberExportNode {
            parent_export,
            member_name,
            file,
            is_live: false,
            member_kind,
            is_public_tagged,
        });
    }

    /// Record a namespace alias chain (destructured namespace member).
    pub fn add_namespace_alias_chain(
        &mut self,
        file: FileId,
        namespace_source: FileId,
        namespace_export: CompactString,
        local_name: CompactString,
        member_name: CompactString,
    ) {
        self.namespace_alias_chains.push(NamespaceAliasChain {
            file,
            namespace_source,
            namespace_export,
            local_name,
            member_name,
        });
    }

    /// Mark a specific member as live.
    pub fn mark_member_live(&mut self, file: FileId, parent_export: &str, member_name: &str) {
        for member in &mut self.member_exports {
            if member.file == file
                && member.parent_export == parent_export
                && member.member_name == member_name
            {
                member.is_live = true;
            }
        }
    }

    /// Mark all members of a parent export as live.
    pub fn mark_all_members_live(&mut self, file: FileId, parent_export: &str) {
        for member in &mut self.member_exports {
            if member.file == file && member.parent_export == parent_export {
                member.is_live = true;
            }
        }
    }

    /// Get dead (unused) members for a given file.
    pub fn dead_members(&self, file: FileId) -> impl Iterator<Item = &MemberExportNode> {
        self.member_exports.iter().filter(move |m| m.file == file && !m.is_live)
    }

    /// Get all members for a specific export.
    pub fn members_for_export(
        &self,
        file: FileId,
        parent_export: &str,
    ) -> impl Iterator<Item = &MemberExportNode> {
        let parent = CompactString::new(parent_export);
        self.member_exports.iter().filter(move |m| m.file == file && m.parent_export == parent)
    }

    /// Check if an export has any same-file references.
    pub fn has_same_file_refs(&self, file: FileId, export_name: &str) -> bool {
        self.same_file_refs.iter().any(|r| r.file == file && r.export_name == export_name)
    }

    /// Propagate liveness through all edges.
    ///
    /// 1. For each import edge, mark the target export as live.
    /// 2. For each reexport edge, if the re-exported name is live in the
    ///    reexporter, mark the original in the source as live (follow chains).
    /// 3. For each member ref, mark the specific member as live.
    /// 4. For each same-file ref, mark the export as live.
    /// 5. For each namespace alias chain, mark the corresponding member as live.
    pub fn propagate_liveness(&mut self) {
        // Step 1: import edges -> mark target exports live.
        let import_targets: Vec<(FileId, CompactString)> =
            self.import_edges.iter().map(|e| (e.source, e.export_name.clone())).collect();
        for (file, name) in import_targets {
            if let Some(node) = self.exports.get_mut(&(file, name)) {
                node.is_live = true;
            }
        }

        // Step 2: reexport edges -> propagate liveness through chains.
        // Iterate until no new liveness is discovered (handles transitive chains).
        loop {
            let mut changed = false;
            for i in 0..self.reexport_edges.len() {
                let reexporter = self.reexport_edges[i].reexporter;
                let exported_name = self.reexport_edges[i].exported_name.clone();
                let source = self.reexport_edges[i].source;
                let original_name = self.reexport_edges[i].original_name.clone();
                let is_star = self.reexport_edges[i].is_star;

                let reexported_is_live = self
                    .exports
                    .get(&(reexporter, exported_name.clone()))
                    .is_some_and(|n| n.is_live);

                if reexported_is_live {
                    if is_star && original_name == "*" && exported_name == "*" {
                        // True star re-export (`export * from`): mark all source exports live.
                        // Namespace re-exports (`export * as Name from`) are handled
                        // by step 3b via member refs instead.
                        let source_keys: Vec<CompactString> = self
                            .exports
                            .iter()
                            .filter(|((f, _), _)| *f == source)
                            .map(|((_, name), _)| name.clone())
                            .collect();
                        for name in source_keys {
                            if let Some(node) = self.exports.get_mut(&(source, name))
                                && !node.is_live
                            {
                                node.is_live = true;
                                changed = true;
                            }
                        }
                    } else if let Some(node) = self.exports.get_mut(&(source, original_name))
                        && !node.is_live
                    {
                        node.is_live = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Step 3: member refs -> mark specific members live.
        let member_targets: Vec<(FileId, CompactString, CompactString)> = self
            .member_refs
            .iter()
            .map(|r| (r.source, r.export_name.clone(), r.member_name.clone()))
            .collect();
        for (file, parent, member) in member_targets {
            for m in &mut self.member_exports {
                if m.file == file && m.parent_export == parent && m.member_name == member {
                    m.is_live = true;
                }
            }
        }

        // Step 3b: member refs on namespace re-exports → mark source exports live.
        // When `MathUtils.add` is accessed and `MathUtils` is `export * as MathUtils from './core'`,
        // mark `add` as live in core.ts.
        for member_ref in &self.member_refs {
            // Find re-export edges where the re-exported name matches the member ref's export_name
            for reexport in &self.reexport_edges {
                if reexport.reexporter == member_ref.source
                    && reexport.exported_name == member_ref.export_name
                    && reexport.is_star
                {
                    // This is a namespace re-export: `export * as Name from './source'`
                    // The member access `Name.member` should make `member` live in the source.
                    if let Some(node) =
                        self.exports.get_mut(&(reexport.source, member_ref.member_name.clone()))
                    {
                        node.is_live = true;
                    }
                }
            }
        }

        // Step 4: same-file refs -> mark exports live.
        let same_file_targets: Vec<(FileId, CompactString)> =
            self.same_file_refs.iter().map(|r| (r.file, r.export_name.clone())).collect();
        for (file, name) in same_file_targets {
            if let Some(node) = self.exports.get_mut(&(file, name)) {
                node.is_live = true;
            }
        }

        // Step 5: namespace alias chains -> mark corresponding members live.
        // When `const { foo } = utils` where `utils` is a namespace import,
        // mark `foo` as a live member of the namespace source.
        let alias_targets: Vec<(FileId, CompactString, CompactString)> = self
            .namespace_alias_chains
            .iter()
            .map(|a| (a.namespace_source, a.namespace_export.clone(), a.member_name.clone()))
            .collect();
        for (source_file, parent, member) in alias_targets {
            // Mark the member export as live.
            for m in &mut self.member_exports {
                if m.file == source_file && m.parent_export == parent && m.member_name == member {
                    m.is_live = true;
                }
            }
            // Also mark the corresponding source export as live (the member
            // itself may be a top-level export in the source file).
            if let Some(node) = self.exports.get_mut(&(source_file, member.clone())) {
                node.is_live = true;
            }
        }
    }
}
