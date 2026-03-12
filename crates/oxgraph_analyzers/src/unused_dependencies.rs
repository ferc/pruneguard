use rustc_hash::{FxHashMap, FxHashSet};

use oxgraph_config::AnalysisSeverity;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_fs::is_docs_path;
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
        if extracted_file.file.role.excluded_from_dead_code_by_default()
            || is_docs_path(&extracted_file.file.relative_path)
            || (profile == EntrypointProfile::Production
                && extracted_file.file.role.is_development_only())
        {
            continue;
        }

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

        let manifest_path = workspace
            .root
            .strip_prefix(&build.discovery.project_root)
            .unwrap_or(&workspace.root)
            .join("package.json")
            .to_string_lossy()
            .to_string();

        for (dependency, dependency_kind) in declared_dependencies(&workspace.manifest, profile) {
            if used.is_some_and(|deps| deps.contains(dependency)) {
                continue;
            }

            let evidence = vec![Evidence {
                kind: "dependency".to_string(),
                file: Some(manifest_path.clone()),
                line: None,
                description: format!(
                    "No reachable file in the active profile resolved to this {dependency_kind}."
                ),
            }];

            findings.push(make_finding(
                "unused-dependency",
                finding_severity,
                FindingCategory::UnusedDependency,
                dependency,
                Some(workspace_name.clone()),
                Some(package_name.clone()),
                format!(
                    "{dependency_kind} `{dependency}` is declared in `{package_name}` but not used by reachable files in the active profile."
                ),
                evidence,
                Some("Remove the dependency or add the missing reference.".to_string()),
                None,
            ));
        }
    }

    findings
}

fn declared_dependencies(
    manifest: &oxgraph_manifest::PackageManifest,
    profile: EntrypointProfile,
) -> Vec<(&str, &'static str)> {
    let mut dependencies = manifest
        .dependencies
        .iter()
        .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "dependency")))
        .chain(
            manifest
                .peer_dependencies
                .iter()
                .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "peer dependency"))),
        )
        .chain(
            manifest
                .optional_dependencies
                .iter()
                .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "optional dependency"))),
        )
        .collect::<Vec<_>>();

    if profile != EntrypointProfile::Production {
        dependencies.extend(
            manifest
                .dev_dependencies
                .iter()
                .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "dev dependency"))),
        );
    }

    dependencies.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
    dependencies.dedup();
    dependencies
}
