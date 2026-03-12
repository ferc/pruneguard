use std::collections::VecDeque;

use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_fs::{FileKind, FileRole};
use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ids::{FileId, Interner, PackageId, WorkspaceId};

/// The module-level dependency graph.
#[derive(Debug)]
pub struct ModuleGraph {
    pub graph: DiGraph<ModuleNode, ModuleEdge>,
    pub file_index: FxHashMap<FileId, NodeIndex>,
    pub package_index: FxHashMap<PackageId, NodeIndex>,
    pub workspace_index: FxHashMap<WorkspaceId, NodeIndex>,
    pub entrypoint_index: FxHashMap<(FileId, EntrypointProfile), NodeIndex>,
    pub external_index: FxHashMap<String, NodeIndex>,
    pub interner: Interner,
}

/// A node in the module graph.
#[derive(Debug, Clone)]
pub enum ModuleNode {
    Workspace {
        id: WorkspaceId,
        name: String,
        path: String,
    },
    Package {
        id: PackageId,
        name: String,
        workspace: Option<String>,
        path: String,
        version: Option<String>,
    },
    File {
        id: FileId,
        path: String,
        relative_path: String,
        workspace: Option<String>,
        package: Option<String>,
        kind: FileKind,
        role: FileRole,
    },
    Entrypoint {
        file: FileId,
        path: String,
        kind: EntrypointKind,
        profile: EntrypointProfile,
        workspace: Option<String>,
        source: String,
    },
    ExternalDependency {
        name: String,
    },
}

/// The kind of entrypoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointKind {
    PackageMain,
    PackageBin,
    PackageExports,
    Explicit,
    FrameworkDetected,
    Convention,
    PackageScript,
}

impl EntrypointKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PackageMain => "package-main",
            Self::PackageBin => "package-bin",
            Self::PackageExports => "package-exports",
            Self::Explicit => "explicit-config",
            Self::FrameworkDetected => "framework-pack",
            Self::Convention => "convention",
            Self::PackageScript => "package-script",
        }
    }
}

/// An edge in the module graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            entrypoint_index: FxHashMap::default(),
            external_index: FxHashMap::default(),
            interner: Interner::default(),
        }
    }

    /// Add a workspace node and return its ID and node index.
    pub fn add_workspace(&mut self, name: &str, path: &str) -> (WorkspaceId, NodeIndex) {
        let id = WorkspaceId(self.interner.intern(name));
        let idx = self.graph.add_node(ModuleNode::Workspace {
            id,
            name: name.to_string(),
            path: path.to_string(),
        });
        self.workspace_index.insert(id, idx);
        (id, idx)
    }

    /// Add a package node and return its ID and node index.
    pub fn add_package(
        &mut self,
        name: &str,
        workspace: Option<&str>,
        path: &str,
        version: Option<&str>,
    ) -> (PackageId, NodeIndex) {
        let id = PackageId(self.interner.intern(name));
        let idx = self.graph.add_node(ModuleNode::Package {
            id,
            name: name.to_string(),
            workspace: workspace.map(ToString::to_string),
            path: path.to_string(),
            version: version.map(ToString::to_string),
        });
        self.package_index.insert(id, idx);
        (id, idx)
    }

    /// Add a file node and return its ID and node index.
    pub fn add_file(
        &mut self,
        path: &str,
        relative_path: &str,
        workspace: Option<&str>,
        package: Option<&str>,
        kind: FileKind,
        role: FileRole,
    ) -> (FileId, NodeIndex) {
        let id = FileId(self.interner.intern(path));
        let idx = self.graph.add_node(ModuleNode::File {
            id,
            path: path.to_string(),
            relative_path: relative_path.to_string(),
            workspace: workspace.map(ToString::to_string),
            package: package.map(ToString::to_string),
            kind,
            role,
        });
        self.file_index.insert(id, idx);
        (id, idx)
    }

    /// Add an entrypoint node for a file.
    pub fn add_entrypoint(
        &mut self,
        file: FileId,
        path: &str,
        kind: EntrypointKind,
        profile: EntrypointProfile,
        workspace: Option<&str>,
        source: &str,
    ) -> NodeIndex {
        let idx = self.graph.add_node(ModuleNode::Entrypoint {
            file,
            path: path.to_string(),
            kind,
            profile,
            workspace: workspace.map(ToString::to_string),
            source: source.to_string(),
        });
        self.entrypoint_index.insert((file, profile), idx);
        idx
    }

    /// Return the external dependency node, creating it when needed.
    pub fn external_dependency_node(&mut self, name: &str) -> NodeIndex {
        if let Some(index) = self.external_index.get(name).copied() {
            return index;
        }
        let idx = self.graph.add_node(ModuleNode::ExternalDependency { name: name.to_string() });
        self.external_index.insert(name.to_string(), idx);
        idx
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

    /// Get the file ID for a path, if it exists.
    pub fn file_id(&self, path: &str) -> Option<FileId> {
        let raw_id = self.interner.lookup(path)?;
        Some(FileId(raw_id))
    }

    /// Get all file nodes.
    pub fn file_nodes(&self) -> Vec<(NodeIndex, &ModuleNode)> {
        self.graph
            .node_indices()
            .filter_map(|index| match &self.graph[index] {
                node @ ModuleNode::File { .. } => Some((index, node)),
                _ => None,
            })
            .collect()
    }

    /// Get the module node for a specific file ID.
    pub fn file_node_by_id(&self, file_id: FileId) -> Option<(NodeIndex, &ModuleNode)> {
        let index = self.file_index.get(&file_id).copied()?;
        Some((index, &self.graph[index]))
    }

    /// Get the file ID associated with an entrypoint node.
    pub fn entrypoint_file_id(&self, index: NodeIndex) -> Option<FileId> {
        match &self.graph[index] {
            ModuleNode::Entrypoint { file, .. } => Some(*file),
            _ => None,
        }
    }

    /// Get all entrypoint nodes for the given profile.
    pub fn entrypoint_nodes(&self, profile: EntrypointProfile) -> Vec<NodeIndex> {
        self.graph
            .node_indices()
            .filter(|index| match &self.graph[*index] {
                ModuleNode::Entrypoint { profile: node_profile, .. } => match profile {
                    EntrypointProfile::Both => true,
                    EntrypointProfile::Production => {
                        *node_profile == EntrypointProfile::Production
                            || *node_profile == EntrypointProfile::Both
                    }
                    EntrypointProfile::Development => {
                        *node_profile == EntrypointProfile::Development
                            || *node_profile == EntrypointProfile::Both
                    }
                },
                _ => false,
            })
            .collect()
    }

    /// Compute the reachable node set from entrypoints for a profile.
    pub fn reachable_nodes(&self, profile: EntrypointProfile) -> FxHashSet<NodeIndex> {
        let mut visited = FxHashSet::default();
        let mut queue: VecDeque<NodeIndex> = self.entrypoint_nodes(profile).into();

        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }

            for edge in self.graph.edges(node) {
                queue.push_back(edge.target());
            }
        }

        visited
    }

    /// Compute the reachable file IDs from entrypoints for a profile.
    pub fn reachable_file_ids(&self, profile: EntrypointProfile) -> FxHashSet<FileId> {
        self.reachable_nodes(profile)
            .into_iter()
            .filter_map(|index| match &self.graph[index] {
                ModuleNode::File { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Compute reverse-reachable node set from a target file.
    pub fn reverse_reachable_nodes_from_file(&self, file_id: FileId) -> FxHashSet<NodeIndex> {
        let Some(start) = self.file_index.get(&file_id).copied() else {
            return FxHashSet::default();
        };

        let mut visited = FxHashSet::default();
        let mut queue = VecDeque::from([start]);

        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }

            for edge in self.graph.edges_directed(node, petgraph::Direction::Incoming) {
                queue.push_back(edge.source());
            }
        }

        visited
    }

    /// Find the shortest forward path from any entrypoint to a target file.
    pub fn shortest_path_to_file(
        &self,
        file_id: FileId,
        profile: EntrypointProfile,
    ) -> Option<Vec<NodeIndex>> {
        let target = self.file_index.get(&file_id).copied()?;
        let mut parent: FxHashMap<NodeIndex, NodeIndex> = FxHashMap::default();
        let mut visited = FxHashSet::default();
        let mut queue: VecDeque<NodeIndex> = self.entrypoint_nodes(profile).into();

        for node in &queue {
            visited.insert(*node);
        }

        while let Some(node) = queue.pop_front() {
            if node == target {
                let mut path = vec![node];
                let mut cursor = node;
                while let Some(prev) = parent.get(&cursor).copied() {
                    path.push(prev);
                    cursor = prev;
                }
                path.reverse();
                return Some(path);
            }

            for edge in self.graph.edges(node) {
                let next = edge.target();
                if visited.insert(next) {
                    parent.insert(next, node);
                    queue.push_back(next);
                }
            }
        }

        None
    }

    /// Return strongly connected file components.
    pub fn strongly_connected_file_components(&self) -> Vec<Vec<NodeIndex>> {
        kosaraju_scc(&self.graph)
            .into_iter()
            .filter(|component| {
                component.iter().any(|index| matches!(self.graph[*index], ModuleNode::File { .. }))
            })
            .collect()
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
