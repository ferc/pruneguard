use petgraph::graph::{DiGraph, NodeIndex};
use rustc_hash::FxHashMap;

use crate::ids::{FileId, Interner, PackageId, WorkspaceId};

/// The module-level dependency graph.
///
/// Nodes represent files, packages, workspaces, entrypoints, and external deps.
/// Edges represent import/require/re-export relationships.
#[derive(Debug)]
pub struct ModuleGraph {
    /// The petgraph directed graph.
    pub graph: DiGraph<ModuleNode, ModuleEdge>,
    /// Map from file path (interned) to node index.
    pub file_index: FxHashMap<FileId, NodeIndex>,
    /// Map from package name (interned) to node index.
    pub package_index: FxHashMap<PackageId, NodeIndex>,
    /// Map from workspace name (interned) to node index.
    pub workspace_index: FxHashMap<WorkspaceId, NodeIndex>,
    /// String interner for paths and names.
    pub interner: Interner,
}

/// A node in the module graph.
#[derive(Debug, Clone)]
pub enum ModuleNode {
    Workspace { id: WorkspaceId, name: String },
    Package { id: PackageId, name: String, workspace: WorkspaceId },
    File { id: FileId, path: String, workspace: WorkspaceId },
    Entrypoint { file: FileId, kind: EntrypointKind },
    ExternalDependency { name: String },
}

/// The kind of entrypoint.
#[derive(Debug, Clone, Copy)]
pub enum EntrypointKind {
    PackageMain,
    PackageBin,
    PackageExports,
    Explicit,
    FrameworkDetected,
    Convention,
}

/// An edge in the module graph.
#[derive(Debug, Clone)]
pub enum ModuleEdge {
    StaticImportValue,
    StaticImportType,
    DynamicImport,
    Require,
    SideEffectImport,
    ReExportNamed,
    ReExportAll,
    EntrypointToFile,
    PackageToEntrypoint,
    FileToDependency,
}

impl ModuleGraph {
    /// Create a new empty module graph.
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            file_index: FxHashMap::default(),
            package_index: FxHashMap::default(),
            workspace_index: FxHashMap::default(),
            interner: Interner::default(),
        }
    }

    /// Add a file node and return its node index.
    pub fn add_file(&mut self, path: &str, workspace: WorkspaceId) -> NodeIndex {
        let id = FileId(self.interner.intern(path));
        let idx = self.graph.add_node(ModuleNode::File { id, path: path.to_string(), workspace });
        self.file_index.insert(id, idx);
        idx
    }

    /// Add a workspace node and return its node index.
    pub fn add_workspace(&mut self, name: &str) -> (WorkspaceId, NodeIndex) {
        let id = WorkspaceId(self.interner.intern(name));
        let idx = self.graph.add_node(ModuleNode::Workspace { id, name: name.to_string() });
        self.workspace_index.insert(id, idx);
        (id, idx)
    }

    /// Add a package node and return its node index.
    pub fn add_package(&mut self, name: &str, workspace: WorkspaceId) -> (PackageId, NodeIndex) {
        let id = PackageId(self.interner.intern(name));
        let idx =
            self.graph.add_node(ModuleNode::Package { id, name: name.to_string(), workspace });
        self.package_index.insert(id, idx);
        (id, idx)
    }

    /// Add an edge between two nodes.
    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: ModuleEdge) {
        self.graph.add_edge(from, to, edge);
    }

    /// Get the node index for a file, if it exists.
    pub fn file_node(&self, path: &str) -> Option<NodeIndex> {
        let raw_id = self.interner.lookup(path)?;
        let id = FileId(raw_id);
        self.file_index.get(&id).copied()
    }

    /// Return the total number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Return the total number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

impl Default for ModuleGraph {
    fn default() -> Self {
        Self::new()
    }
}
