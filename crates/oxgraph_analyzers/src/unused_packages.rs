use rustc_hash::FxHashSet;

use oxgraph_config::AnalysisSeverity;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_fs::is_docs_path;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Evidence, Finding, FindingCategory, FindingConfidence};

use crate::{make_finding, severity};

/// Find workspace packages with no reachable files.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let reachable = build.module_graph.reachable_file_ids(profile);
    let mut reachable_workspaces = FxHashSet::default();
    let mut candidate_workspaces = FxHashSet::default();

    for extracted_file in &build.files {
        if extracted_file.file.role.excluded_from_dead_code_by_default()
            || is_docs_path(&extracted_file.file.relative_path)
            || (profile == EntrypointProfile::Production
                && extracted_file.file.role.is_development_only())
        {
            continue;
        }

        if let Some(workspace) = &extracted_file.file.workspace {
            candidate_workspaces.insert(workspace.clone());
        }

        let Some(file_id) = build
            .module_graph
            .file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };

        if reachable.contains(&file_id)
            && let Some(workspace) = &extracted_file.file.workspace
        {
            reachable_workspaces.insert(workspace.clone());
        }
    }

    for seed in &build.entrypoint_seeds {
        let active = match profile {
            EntrypointProfile::Both => true,
            EntrypointProfile::Production => {
                seed.profile == EntrypointProfile::Production || seed.profile == EntrypointProfile::Both
            }
            EntrypointProfile::Development => {
                seed.profile == EntrypointProfile::Development || seed.profile == EntrypointProfile::Both
            }
        };
        if active
            && let Some(workspace) = &seed.workspace
        {
            candidate_workspaces.insert(workspace.clone());
        }
    }

    let mut findings = Vec::new();
    for workspace in build.discovery.workspaces.values() {
        if reachable_workspaces.contains(&workspace.name) || !candidate_workspaces.contains(&workspace.name) {
            continue;
        }

        let package_name = workspace
            .manifest
            .name
            .clone()
            .unwrap_or_else(|| workspace.name.clone());

        findings.push(make_finding(
            "unused-package",
            finding_severity,
            FindingCategory::UnusedPackage,
            FindingConfidence::Medium,
            &package_name,
            Some(workspace.name.clone()),
            Some(package_name.clone()),
            format!(
                "Package `{package_name}` has no reachable source files or entrypoints."
            ),
            vec![Evidence {
                kind: "package".to_string(),
                file: Some(
                    workspace
                        .root
                        .strip_prefix(&build.discovery.project_root)
                        .unwrap_or(&workspace.root)
                        .to_string_lossy()
                        .to_string(),
                ),
                line: None,
                description: "No active entrypoint or reachable package edge reaches this workspace package.".to_string(),
            }],
            Some("Remove the package or wire it into the reachable graph.".to_string()),
            None,
        ));
    }

    findings
}
