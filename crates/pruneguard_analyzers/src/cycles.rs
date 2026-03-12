use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use pruneguard_config::AnalysisSeverity;
use pruneguard_graph::{GraphBuildResult, ModuleNode};
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence};
use rustc_hash::FxHashSet;

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

        let chain = minimal_cycle_chain(build, &component).unwrap_or_else(|| files.clone());
        let subject = chain.join(" -> ");
        let primary = files.first().cloned().unwrap_or_default();
        let break_edges = chain
            .windows(2)
            .map(|window| format!("{} -> {}", window[0], window[1]))
            .collect::<Vec<_>>();
        findings.push(make_finding(
            "cycle",
            finding_severity,
            FindingCategory::Cycle,
            FindingConfidence::High,
            &subject,
            None,
            None,
            format!("Detected a dependency cycle involving {} files.", files.len()),
            vec![
                Evidence {
                    kind: "cycle".to_string(),
                    file: Some(primary),
                    line: None,
                    description: format!("Cycle chain: {}", chain.join(" -> ")),
                },
                Evidence {
                    kind: "cycle".to_string(),
                    file: None,
                    line: None,
                    description: format!("Candidate break edges: {}.", break_edges.join(", ")),
                },
            ],
            Some("Break one edge in the cycle to restore acyclic reachability.".to_string()),
            None,
        ));
    }

    findings
}

fn minimal_cycle_chain(build: &GraphBuildResult, component: &[NodeIndex]) -> Option<Vec<String>> {
    let component_set = component.iter().copied().collect::<FxHashSet<_>>();
    let &start = component
        .iter()
        .find(|index| matches!(build.module_graph.graph[**index], ModuleNode::File { .. }))?;
    let mut stack = vec![(start, vec![start])];

    while let Some((node, path)) = stack.pop() {
        for edge in build.module_graph.graph.edges(node) {
            let next = edge.target();
            if !component_set.contains(&next)
                || !matches!(build.module_graph.graph[next], ModuleNode::File { .. })
            {
                continue;
            }
            if next == start && path.len() > 1 {
                let mut cycle = path
                    .iter()
                    .filter_map(|index| match &build.module_graph.graph[*index] {
                        ModuleNode::File { relative_path, .. } => Some(relative_path.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if let Some(first) = cycle.first().cloned() {
                    cycle.push(first);
                }
                return Some(cycle);
            }
            if path.contains(&next) {
                continue;
            }

            let mut next_path = path.clone();
            next_path.push(next);
            stack.push((next, next_path));
        }
    }

    None
}
