use rustc_hash::FxHashSet;

use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_graph::{FileId, GraphBuildResult, ModuleNode};

/// Reverse-reachability result for a target file.
#[derive(Debug, Default)]
#[allow(clippy::struct_field_names)]
pub struct ImpactAnalysis {
    pub affected_entrypoints: Vec<String>,
    pub affected_packages: Vec<String>,
    pub affected_files: Vec<String>,
}

/// Compute reverse reachability from a target file.
pub fn analyze(
    build: &GraphBuildResult,
    file_id: FileId,
    profile: EntrypointProfile,
) -> ImpactAnalysis {
    let reverse = build.module_graph.reverse_reachable_nodes_from_file(file_id);
    let active_entrypoints = build
        .module_graph
        .entrypoint_nodes(profile)
        .into_iter()
        .collect::<FxHashSet<_>>();

    let mut affected_entrypoints = Vec::new();
    let mut affected_packages = FxHashSet::default();
    let mut affected_files = Vec::new();

    for node in reverse {
        match &build.module_graph.graph[node] {
            ModuleNode::Entrypoint { path, .. } if active_entrypoints.contains(&node) => {
                affected_entrypoints.push(path.clone());
            }
            ModuleNode::Package { name, .. } => {
                affected_packages.insert(name.clone());
            }
            ModuleNode::File { relative_path, package, .. } => {
                affected_files.push(relative_path.clone());
                if let Some(package) = package {
                    affected_packages.insert(package.clone());
                }
            }
            _ => {}
        }
    }

    affected_entrypoints.sort();
    affected_entrypoints.dedup();
    affected_files.sort();
    affected_files.dedup();
    let mut affected_packages = affected_packages.into_iter().collect::<Vec<_>>();
    affected_packages.sort();

    ImpactAnalysis { affected_entrypoints, affected_packages, affected_files }
}
