use std::collections::BTreeMap;

use petgraph::visit::EdgeRef;
use rustc_hash::FxHashSet;

use pruneguard_config::PruneguardConfig;
use pruneguard_graph::{GraphBuildResult, ModuleEdge, ModuleNode};
use pruneguard_report::{
    FindingConfidence, Hotspot, OwnershipHint, SuggestRulesReport, SuggestedRule,
    SuggestedRuleKind, SuggestedTag,
};

/// Threshold: minimum cross-package edge count before suggesting a boundary rule.
const BOUNDARY_EDGE_THRESHOLD: usize = 3;

/// Threshold: minimum files in a directory pattern to suggest a tag.
const TAG_FILE_THRESHOLD: usize = 3;

/// Threshold: minimum total edges to flag a file as a hotspot.
const HOTSPOT_EDGE_THRESHOLD: usize = 5;

/// Analyze the graph and configuration to suggest governance rules.
pub fn suggest_rules(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
) -> SuggestRulesReport {
    let mut report = SuggestRulesReport::default();

    suggest_boundary_rules(graph, config, &mut report);
    suggest_tags(graph, &mut report);
    suggest_hotspots(graph, &mut report);
    suggest_ownership_hints(graph, config, &mut report);

    if report.suggested_rules.is_empty()
        && report.tags.is_empty()
        && report.hotspots.is_empty()
        && report.ownership_hints.is_empty()
    {
        report.rationale.push(
            "No governance suggestions were generated. The project may be too small or \
             already well-configured."
                .to_string(),
        );
    }

    report
}

/// Suggest forbidden-dependency boundary rules by analyzing cross-package import edges.
fn suggest_boundary_rules(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    // Already has rules configured -- skip boundary suggestions.
    if config.rules.is_some() {
        report.rationale.push(
            "Boundary rules already configured; skipping boundary suggestions.".to_string(),
        );
        return;
    }

    // Count cross-package edges: (source_package, target_package) -> count.
    let mut cross_package_edges: BTreeMap<(String, String), usize> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            package: Some(source_package),
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }

            let target = edge.target();
            if let ModuleNode::File {
                package: Some(target_package),
                ..
            } = &graph.module_graph.graph[target]
                && source_package != target_package
            {
                *cross_package_edges
                    .entry((source_package.clone(), target_package.clone()))
                    .or_insert(0) += 1;
            }
        }
    }

    // Find heavily-crossed boundaries.
    for ((source, target), count) in &cross_package_edges {
        if *count < BOUNDARY_EDGE_THRESHOLD {
            continue;
        }

        let name = format!("forbid-{source}-to-{target}");
        let description = format!(
            "Package `{source}` imports from `{target}` across {count} edges. \
             Consider a forbidden rule if this crosses an intended boundary."
        );
        let confidence = if *count >= BOUNDARY_EDGE_THRESHOLD * 3 {
            FindingConfidence::High
        } else if *count >= BOUNDARY_EDGE_THRESHOLD * 2 {
            FindingConfidence::Medium
        } else {
            FindingConfidence::Low
        };

        let config_fragment = serde_json::json!({
            "rules": [{
                "name": name,
                "severity": "error",
                "from": { "package": source },
                "to": { "package": target },
                "forbidden": true
            }]
        });

        report.suggested_rules.push(SuggestedRule {
            kind: SuggestedRuleKind::Forbidden,
            name,
            description,
            config_fragment,
            confidence,
            evidence: vec![format!("{count} cross-package edges from `{source}` to `{target}`")],
        });
    }

    if !cross_package_edges.is_empty() && report.suggested_rules.is_empty() {
        report.rationale.push(
            "Cross-package imports exist but none exceed the threshold for a boundary suggestion."
                .to_string(),
        );
    }
}

/// Suggest tags by grouping files into common directory patterns.
fn suggest_tags(graph: &GraphBuildResult, report: &mut SuggestRulesReport) {
    // Group files by their first two path segments (e.g. "src/components").
    let mut dir_counts: BTreeMap<String, usize> = BTreeMap::new();

    for file in &graph.files {
        let relative = file.file.relative_path.to_string_lossy();
        if let Some(dir) = extract_tag_directory(&relative) {
            *dir_counts.entry(dir).or_insert(0) += 1;
        }
    }

    for (dir, count) in &dir_counts {
        if *count < TAG_FILE_THRESHOLD {
            continue;
        }

        let tag_name = dir.replace(['/', '\\'], "-");
        let glob = format!("{dir}/**");
        let rationale = format!(
            "{count} files match the pattern `{glob}`. A tag would allow \
             writing boundary rules for this group."
        );

        report.tags.push(SuggestedTag {
            name: tag_name,
            glob,
            rationale,
        });
    }
}

/// Detect files with high fan-in / fan-out as hotspots.
fn suggest_hotspots(graph: &GraphBuildResult, report: &mut SuggestRulesReport) {
    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            relative_path,
            package: source_package,
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        let mut outgoing = 0usize;
        let mut cross_package_out = 0usize;

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            outgoing += 1;
            if let ModuleNode::File {
                package: Some(target_pkg),
                ..
            } = &graph.module_graph.graph[edge.target()]
                && source_package.as_ref() != Some(target_pkg)
            {
                cross_package_out += 1;
            }
        }

        let mut incoming = 0usize;
        let mut cross_package_in = 0usize;

        for edge in graph
            .module_graph
            .graph
            .edges_directed(node_idx, petgraph::Direction::Incoming)
        {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            incoming += 1;
            if let ModuleNode::File {
                package: Some(importer_pkg),
                ..
            } = &graph.module_graph.graph[edge.source()]
                && source_package.as_ref() != Some(importer_pkg)
            {
                cross_package_in += 1;
            }
        }

        let total_edges = incoming + outgoing;
        let cross_package_imports = cross_package_in + cross_package_out;

        if total_edges < HOTSPOT_EDGE_THRESHOLD {
            continue;
        }

        let suggestion = if cross_package_imports > total_edges / 2 {
            format!(
                "`{relative_path}` has high cross-package traffic ({cross_package_imports} \
                 cross-package of {total_edges} total edges). Consider extracting a shared \
                 package or adding boundary rules."
            )
        } else {
            format!(
                "`{relative_path}` has high edge count ({total_edges} total). \
                 Consider whether ownership assignment or boundary rules would help."
            )
        };

        report.hotspots.push(Hotspot {
            file: relative_path.clone(),
            cross_package_imports,
            cross_owner_imports: 0,
            incoming_edges: incoming,
            outgoing_edges: outgoing,
            suggestion,
        });
    }

    // Sort hotspots by total edges descending.
    report
        .hotspots
        .sort_by(|a, b| {
            let total_a = a.incoming_edges + a.outgoing_edges;
            let total_b = b.incoming_edges + b.outgoing_edges;
            total_b.cmp(&total_a)
        });

    // Keep only the top 20 hotspots.
    report.hotspots.truncate(20);
}

/// Suggest ownership hints by detecting directory clusters with high cross-package edges.
fn suggest_ownership_hints(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    // If ownership is already fully configured, skip.
    if config.ownership.is_some() {
        report.rationale.push(
            "Ownership configuration already present; skipping ownership hints.".to_string(),
        );
        return;
    }

    // Group cross-package edges by directory prefix of the source file.
    let mut dir_cross_edges: BTreeMap<String, usize> = BTreeMap::new();
    let mut dir_packages: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            relative_path,
            package: Some(source_package),
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        let dir = directory_prefix(relative_path);

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            if let ModuleNode::File {
                package: Some(target_package),
                ..
            } = &graph.module_graph.graph[edge.target()]
                && source_package != target_package
            {
                *dir_cross_edges.entry(dir.clone()).or_insert(0) += 1;
                dir_packages
                    .entry(dir.clone())
                    .or_default()
                    .insert(target_package.clone());
            }
        }
    }

    for (dir, cross_edges) in &dir_cross_edges {
        if *cross_edges < BOUNDARY_EDGE_THRESHOLD {
            continue;
        }

        let target_packages = dir_packages.get(dir).map_or(0, FxHashSet::len);
        let suggested_owner = format!("team-{dir}");
        let path_glob = format!("{dir}/**");
        let rationale = format!(
            "Directory `{dir}` has {cross_edges} cross-package import edges \
             spanning {target_packages} other package(s). Assigning an owner \
             would enable ownership-boundary enforcement."
        );

        report.ownership_hints.push(OwnershipHint {
            path_glob,
            suggested_owner,
            cross_team_edges: *cross_edges,
            rationale,
        });
    }
}

/// Check if an edge represents an import (not structural edges like entrypoint-to-file).
const fn is_import_edge(edge: ModuleEdge) -> bool {
    matches!(
        edge,
        ModuleEdge::StaticImportValue
            | ModuleEdge::StaticImportType
            | ModuleEdge::DynamicImport
            | ModuleEdge::Require
            | ModuleEdge::SideEffectImport
            | ModuleEdge::ReExportNamed
            | ModuleEdge::ReExportAll
    )
}

/// Extract a directory pattern suitable for tag naming.
/// Takes the first two path segments if the path has at least 3 segments.
fn extract_tag_directory(relative_path: &str) -> Option<String> {
    let parts: Vec<&str> = relative_path.split('/').collect();
    if parts.len() >= 3 {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
}

/// Extract the first directory segment of a relative path.
fn directory_prefix(relative_path: &str) -> String {
    relative_path
        .split('/')
        .next()
        .unwrap_or(relative_path)
        .to_string()
}
