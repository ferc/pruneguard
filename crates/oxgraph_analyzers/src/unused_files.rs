use oxgraph_config::AnalysisSeverity;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_fs::is_docs_path;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Evidence, Finding, FindingCategory, FindingConfidence};
use petgraph::visit::EdgeRef;

use crate::{make_finding, severity};

/// Find tracked files that are unreachable from the active entrypoints.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let reachable = build.module_graph.reachable_file_ids(profile);
    let mut findings = Vec::new();

    for extracted_file in &build.files {
        if extracted_file.file.role.excluded_from_dead_code_by_default()
            || is_docs_path(&extracted_file.file.relative_path)
            || is_ambient_declaration_file(&extracted_file.file.relative_path)
            || (profile == EntrypointProfile::Production
                && extracted_file.file.role.is_development_only())
        {
            continue;
        }

        let Some(file_id) = build
            .module_graph
            .file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };

        if reachable.contains(&file_id) {
            continue;
        }

        let evidence = vec![Evidence {
            kind: "reachability".to_string(),
            file: Some(extracted_file.file.relative_path.to_string_lossy().to_string()),
            line: None,
            description: "No active entrypoint reaches this file.".to_string(),
        }];
        let confidence = if has_zero_incoming_edges(build, file_id)
            && !has_unresolved_neighbors(extracted_file)
        {
            FindingConfidence::High
        } else {
            FindingConfidence::Medium
        };

        findings.push(make_finding(
            "unused-file",
            finding_severity,
            FindingCategory::UnusedFile,
            confidence,
            extracted_file.file.relative_path.to_string_lossy(),
            extracted_file.file.workspace.clone(),
            extracted_file.file.package.clone(),
            format!(
                "File `{}` is unreachable from the active entrypoints.",
                extracted_file.file.relative_path.to_string_lossy()
            ),
            evidence,
            Some("Remove the file or add an entrypoint/reference that keeps it live.".to_string()),
            None,
        ));
    }

    findings
}

fn has_zero_incoming_edges(build: &GraphBuildResult, file_id: oxgraph_graph::FileId) -> bool {
    let Some((node_index, _)) = build.module_graph.file_node_by_id(file_id) else {
        return false;
    };

    !build
        .module_graph
        .graph
        .edges_directed(node_index, petgraph::Direction::Incoming)
        .any(|edge| {
            matches!(
                build.module_graph.graph[edge.source()],
                oxgraph_graph::ModuleNode::File { .. } | oxgraph_graph::ModuleNode::Entrypoint { .. }
            )
        })
}

fn has_unresolved_neighbors(file: &oxgraph_extract::ExtractedFile) -> bool {
    file.resolved_imports
        .iter()
        .chain(&file.resolved_reexports)
        .any(|edge| matches!(edge.outcome, oxgraph_resolver::ResolutionOutcome::Unresolved))
}

fn is_ambient_declaration_file(path: &std::path::Path) -> bool {
    let path = path.to_string_lossy();
    path.ends_with(".d.ts")
        || path.ends_with(".d.mts")
        || path.ends_with(".d.cts")
        || path.ends_with("env.d.ts")
        || path.ends_with("vite-env.d.ts")
}
