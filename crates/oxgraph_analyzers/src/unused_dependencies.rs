use rustc_hash::{FxHashMap, FxHashSet};

use oxgraph_config::AnalysisSeverity;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Evidence, Finding, FindingCategory};

use crate::{make_finding, severity};

/// Find declared package dependencies that are never referenced by reachable files.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let reachable = build.module_graph.reachable_file_ids(profile);
    let mut used_by_workspace: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();

    for extracted_file in &build.files {
        let Some(workspace) = &extracted_file.file.workspace else {
            continue;
        };
        let Some(file_id) = build
            .module_graph
            .file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };

        if !reachable.contains(&file_id) {
            continue;
        }

        let used = used_by_workspace.entry(workspace.clone()).or_default();
        for dependency in &extracted_file.external_dependencies {
            used.insert(dependency.clone());
        }
    }

    let mut findings = Vec::new();
    for workspace in build.discovery.workspaces.values() {
        let workspace_name = workspace.name.clone();
        let package_name = workspace
            .manifest
            .name
            .clone()
            .unwrap_or_else(|| workspace_name.clone());
        let used = used_by_workspace.get(&workspace_name);

        let declared_dependencies = workspace
            .manifest
            .production_dependencies()
            .chain(workspace.manifest.dev_dependencies_names())
            .collect::<Vec<_>>();

        for dependency in declared_dependencies {
            if used.is_some_and(|deps| deps.contains(dependency)) {
                continue;
            }

            let evidence = vec![Evidence {
                kind: "dependency".to_string(),
                file: Some(
                    workspace
                        .root
                        .strip_prefix(&build.discovery.project_root)
                        .unwrap_or(&workspace.root)
                        .join("package.json")
                        .to_string_lossy()
                        .to_string(),
                ),
                line: None,
                description: "No reachable file resolved to this dependency.".to_string(),
            }];

            findings.push(make_finding(
                "unused-dependency",
                finding_severity,
                FindingCategory::UnusedDependency,
                dependency,
                Some(workspace_name.clone()),
                Some(package_name.clone()),
                format!(
                    "Dependency `{dependency}` is declared in `{package_name}` but not used by reachable files."
                ),
                evidence,
                Some("Remove the dependency or add the missing reference.".to_string()),
                None,
            ));
        }
    }

    findings
}
