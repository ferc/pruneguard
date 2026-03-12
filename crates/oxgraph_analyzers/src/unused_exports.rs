use oxgraph_config::AnalysisSeverity;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_graph::{GraphBuildResult, ModuleNode};
use oxgraph_report::{Evidence, Finding, FindingCategory};

use crate::{make_finding, severity};

/// Find exports that are never consumed by reachable imports or re-exports.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let reachable_files = build.module_graph.reachable_file_ids(profile);
    let mut symbol_graph = build.symbol_graph.clone();

    for import_edge in symbol_graph.import_edges.clone() {
        if !reachable_files.contains(&import_edge.importer) {
            continue;
        }

        if import_edge.export_name == "*" {
            symbol_graph.mark_all_file_exports_live(import_edge.source, Some(import_edge.is_type));
        } else {
            symbol_graph.mark_live(import_edge.source, &import_edge.export_name);
        }
    }

    for reexport_edge in symbol_graph.reexport_edges.clone() {
        if !reachable_files.contains(&reexport_edge.reexporter) {
            continue;
        }

        if reexport_edge.is_star {
            symbol_graph.mark_all_file_exports_live(reexport_edge.source, None);
        } else {
            symbol_graph.mark_live(reexport_edge.source, &reexport_edge.original_name);
        }
    }

    let mut findings = Vec::new();
    for export in symbol_graph.dead_exports() {
        if !reachable_files.contains(&export.file) {
            continue;
        }

        let Some((_, ModuleNode::File { relative_path, workspace, package, .. })) = build
            .module_graph
            .file_node_by_id(export.file)
        else {
            continue;
        };

        let subject = format!("{relative_path}#{}", export.name);
        findings.push(make_finding(
            "unused-export",
            finding_severity,
            FindingCategory::UnusedExport,
            &subject,
            workspace.clone(),
            package.clone(),
            format!("Export `{}` from `{relative_path}` is never consumed.", export.name),
            vec![Evidence {
                kind: if export.is_type { "type-export" } else { "value-export" }.to_string(),
                file: Some(relative_path.clone()),
                line: None,
                description: "No reachable import or re-export chain marks this export as live.".to_string(),
            }],
            Some("Remove the export or reference it from a reachable module.".to_string()),
            None,
        ));
    }

    findings
}
