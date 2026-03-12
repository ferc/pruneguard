use oxgraph_config::AnalysisSeverity;
use oxgraph_graph::{GraphBuildResult, ModuleNode};
use oxgraph_report::{Evidence, Finding, FindingCategory};

use crate::{make_finding, severity};

/// Find strongly connected components in the file dependency graph.
pub fn analyze(build: &GraphBuildResult, level: AnalysisSeverity) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    for component in build.module_graph.strongly_connected_file_components() {
        let mut files = component
            .iter()
            .filter_map(|index| match &build.module_graph.graph[*index] {
                ModuleNode::File { relative_path, .. } => Some(relative_path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        files.sort();
        files.dedup();

        if files.len() <= 1 {
            continue;
        }

        let subject = files.join(" -> ");
        let primary = files.first().cloned().unwrap_or_default();
        findings.push(make_finding(
            "cycle",
            finding_severity,
            FindingCategory::Cycle,
            &subject,
            None,
            None,
            format!("Detected a dependency cycle involving {} files.", files.len()),
            vec![Evidence {
                kind: "cycle".to_string(),
                file: Some(primary),
                line: None,
                description: format!("Cycle chain: {}", files.join(" -> ")),
            }],
            Some("Break one edge in the cycle to restore acyclic reachability.".to_string()),
            None,
        ));
    }

    findings
}
