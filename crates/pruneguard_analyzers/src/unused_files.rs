use petgraph::visit::EdgeRef;
use pruneguard_config::AnalysisSeverity;
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_fs::is_docs_path;
use pruneguard_graph::GraphBuildResult;
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence};

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

    // Compute global unresolved pressure: if the whole graph has many unresolved
    // specifiers we lower confidence uniformly.
    let global_unresolved = count_global_unresolved(build);
    let global_resolved = count_global_resolved(build);
    #[allow(clippy::cast_precision_loss)]
    let global_pressure_pct = if global_resolved + global_unresolved > 0 {
        (global_unresolved as f64 / (global_resolved + global_unresolved) as f64) * 100.0
    } else {
        0.0
    };

    let mut findings = Vec::new();

    for extracted_file in &build.files {
        if extracted_file.file.role.excluded_from_dead_code_by_default()
            || is_docs_path(&extracted_file.file.relative_path)
            || is_ambient_declaration_file(&extracted_file.file.relative_path)
            || is_global_augmentation_file(&extracted_file.file.relative_path)
            || (profile == EntrypointProfile::Production
                && extracted_file.file.role.is_development_only())
        {
            continue;
        }

        let Some(file_id) = build.module_graph.file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };

        if reachable.contains(&file_id) {
            continue;
        }

        let mut evidence = vec![Evidence {
            kind: "reachability".to_string(),
            file: Some(extracted_file.file.relative_path.to_string_lossy().to_string()),
            line: None,
            description: "No active entrypoint reaches this file.".to_string(),
        }];
        let file_unresolved = count_unresolved_specifiers(extracted_file);
        let file_unresolved_benign = count_benign_unresolved(extracted_file);
        // Only count genuinely-missed specifiers toward the pressure threshold.
        let effective_unresolved = file_unresolved.saturating_sub(file_unresolved_benign);

        let confidence = if effective_unresolved >= 5 || global_pressure_pct > 15.0 {
            // Many unresolved specifiers locally or globally — high chance of false positive.
            FindingConfidence::Low
        } else if has_zero_incoming_edges(build, file_id) && effective_unresolved == 0 {
            // Zero incoming edges and zero unresolved specifiers — truly unreachable.
            // But demote to Medium if global pressure is notable.
            if global_pressure_pct > 5.0 {
                FindingConfidence::Medium
            } else {
                FindingConfidence::High
            }
        } else {
            // Some unresolved specifiers (< 5) or has some incoming edges.
            FindingConfidence::Medium
        };
        if effective_unresolved >= 3 {
            evidence.push(Evidence {
                kind: "unresolved-pressure".to_string(),
                file: Some(extracted_file.file.relative_path.to_string_lossy().to_string()),
                line: None,
                description: format!(
                    "{effective_unresolved} unresolved specifiers may affect accuracy of this finding"
                ),
            });
        }

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

fn has_zero_incoming_edges(build: &GraphBuildResult, file_id: pruneguard_graph::FileId) -> bool {
    let Some((node_index, _)) = build.module_graph.file_node_by_id(file_id) else {
        return false;
    };

    !build.module_graph.graph.edges_directed(node_index, petgraph::Direction::Incoming).any(
        |edge| {
            matches!(
                build.module_graph.graph[edge.source()],
                pruneguard_graph::ModuleNode::File { .. }
                    | pruneguard_graph::ModuleNode::Entrypoint { .. }
            )
        },
    )
}

fn count_unresolved_specifiers(file: &pruneguard_extract::ExtractedFile) -> usize {
    file.resolved_imports
        .iter()
        .chain(&file.resolved_reexports)
        .filter(|edge| matches!(edge.outcome, pruneguard_resolver::ResolutionOutcome::Unresolved))
        .count()
}

/// Count unresolved specifiers that are "benign" — i.e. they are asset imports,
/// externalized built-ins, or unsupported specifiers that should not count toward
/// the unresolved-pressure threshold.
fn count_benign_unresolved(file: &pruneguard_extract::ExtractedFile) -> usize {
    file.resolved_imports
        .iter()
        .chain(&file.resolved_reexports)
        .filter(|edge| {
            matches!(edge.outcome, pruneguard_resolver::ResolutionOutcome::Unresolved)
                && matches!(
                    edge.unresolved_reason,
                    Some(
                        pruneguard_resolver::UnresolvedReason::UnsupportedSpecifier
                            | pruneguard_resolver::UnresolvedReason::Externalized
                    )
                )
        })
        .count()
}

/// Count global unresolved specifiers across the entire build.
const fn count_global_unresolved(build: &GraphBuildResult) -> usize {
    build.stats.unresolved_specifiers
}

/// Count global resolved specifiers across the entire build.
const fn count_global_resolved(build: &GraphBuildResult) -> usize {
    build.stats.files_resolved
}

fn is_ambient_declaration_file(path: &std::path::Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.ends_with(".d.ts")
        || path_str.ends_with(".d.mts")
        || path_str.ends_with(".d.cts")
}

/// Exclude files that are global augmentation declarations or environment shims.
/// These are genuine source artifacts but should not be flagged as unused files
/// because they augment the global scope rather than being imported.
fn is_global_augmentation_file(path: &std::path::Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // Common global augmentation files.
    matches!(
        file_name,
        "env.d.ts"
            | "vite-env.d.ts"
            | "global.d.ts"
            | "globals.d.ts"
            | "declarations.d.ts"
            | "types.d.ts"
            | "ambient.d.ts"
            | "shims.d.ts"
            | "react-app-env.d.ts"
            | "next-env.d.ts"
    )
}
