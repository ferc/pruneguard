use rustc_hash::FxHashMap;

use pruneguard_config::AnalysisSeverity;
use pruneguard_graph::{FileId, GraphBuildResult, ModuleNode};
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence};

use crate::{make_finding, severity};

/// Detect the same symbol exported from multiple paths (barrel re-export
/// collisions).
///
/// When a symbol is re-exported through several barrel files, consumers may
/// import it via different paths, leading to duplicated module instances,
/// bundle bloat, and confusing import suggestions.
///
/// # Algorithm
///
/// 1. Walk all `reexport_edges` in the symbol graph.
/// 2. For each re-export edge, record the mapping from the *origin* file+symbol
///    (the `source` file and `original_name`) to the *re-exporter* file and the
///    name it is re-exported under.
/// 3. Group entries by `(source_file, original_name)`.  Any origin symbol that
///    is publicly reachable via more than one re-export path is a collision.
/// 4. Emit a finding for each collision.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    // Map: (origin file, original export name) → Vec<(reexporter file, exported name)>
    let mut origin_map: FxHashMap<(FileId, String), Vec<(FileId, String)>> = FxHashMap::default();

    for edge in &build.symbol_graph.reexport_edges {
        // Skip star re-exports — they forward the entire namespace, not a
        // specific named symbol, so they don't create a discrete duplicate path
        // for a single symbol.
        if edge.is_star {
            continue;
        }

        origin_map
            .entry((edge.source, edge.original_name.to_string()))
            .or_default()
            .push((edge.reexporter, edge.exported_name.to_string()));
    }

    let mut findings = Vec::new();

    for ((origin_file, original_name), reexporters) in &origin_map {
        // Only emit when there are at least two distinct re-export paths.
        if reexporters.len() < 2 {
            continue;
        }

        // Deduplicate: the same reexporter re-exporting under the same name
        // (e.g. from two identical edges) should not inflate the count.
        let mut unique_paths: Vec<&(FileId, String)> = reexporters.iter().collect();
        unique_paths.sort_by(|a, b| {
            a.0 .0.cmp(&b.0 .0).then_with(|| a.1.cmp(&b.1))
        });
        unique_paths.dedup();

        if unique_paths.len() < 2 {
            continue;
        }

        // Resolve origin file path for the finding subject.
        let origin_relative_path = resolve_relative_path(build, *origin_file)
            .unwrap_or_else(|| format!("<file:{}>", origin_file.0));
        let (origin_workspace, origin_package) = resolve_workspace_package(build, *origin_file);

        // Build human-readable re-export path list and evidence.
        let mut path_descriptions = Vec::with_capacity(unique_paths.len());
        let mut evidence = Vec::with_capacity(unique_paths.len());

        for &&(reexporter_file, ref exported_name) in &unique_paths {
            let reexporter_path = resolve_relative_path(build, reexporter_file)
                .unwrap_or_else(|| format!("<file:{}>", reexporter_file.0));

            let path_desc = if *exported_name == *original_name {
                format!("`{reexporter_path}`")
            } else {
                format!("`{reexporter_path}` (as `{exported_name}`)")
            };
            path_descriptions.push(path_desc);

            evidence.push(Evidence {
                kind: "reexport-path".to_string(),
                file: Some(reexporter_path),
                line: None,
                description: format!(
                    "Re-exports `{original_name}` as `{exported_name}`"
                ),
            });
        }

        let paths_joined = path_descriptions.join(", ");
        let message = format!(
            "Symbol `{original_name}` is re-exported from multiple paths: {paths_joined}"
        );

        findings.push(make_finding(
            "duplicate-export",
            finding_severity,
            FindingCategory::DuplicateExport,
            FindingConfidence::Medium,
            &origin_relative_path,
            origin_workspace,
            origin_package,
            message,
            evidence,
            Some(
                "Consolidate re-exports so each symbol is available from a single public path."
                    .to_string(),
            ),
            None,
        ));
    }

    findings
}

/// Resolve a `FileId` to its relative path string via the module graph.
fn resolve_relative_path(build: &GraphBuildResult, file_id: FileId) -> Option<String> {
    let (_, node) = build.module_graph.file_node_by_id(file_id)?;
    match node {
        ModuleNode::File { relative_path, .. } => Some(relative_path.clone()),
        _ => None,
    }
}

/// Resolve a `FileId` to its workspace and package, if available.
fn resolve_workspace_package(
    build: &GraphBuildResult,
    file_id: FileId,
) -> (Option<String>, Option<String>) {
    let Some((_, node)) = build.module_graph.file_node_by_id(file_id) else {
        return (None, None);
    };
    match node {
        ModuleNode::File { workspace, package, .. } => (workspace.clone(), package.clone()),
        _ => (None, None),
    }
}
