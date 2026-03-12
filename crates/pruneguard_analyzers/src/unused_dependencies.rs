use rustc_hash::{FxHashMap, FxHashSet};

use pruneguard_config::AnalysisSeverity;
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_fs::is_docs_path;
use pruneguard_graph::GraphBuildResult;
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence};

use crate::{make_finding, severity};

/// Find declared package dependencies that are never referenced by reachable files.
#[allow(clippy::too_many_lines)]
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let reachable_prod = build.module_graph.reachable_file_ids(EntrypointProfile::Production);
    let reachable_dev = build.module_graph.reachable_file_ids(EntrypointProfile::Development);
    let mut used_prod_by_workspace: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    let mut used_dev_by_workspace: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();

    for extracted_file in &build.files {
        if extracted_file.file.role.excluded_from_dead_code_by_default()
            || is_docs_path(&extracted_file.file.relative_path)
        {
            continue;
        }

        let Some(workspace) = &extracted_file.file.workspace else {
            continue;
        };
        let Some(file_id) = build.module_graph.file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };

        let is_prod_reachable =
            reachable_prod.contains(&file_id) && !extracted_file.file.role.is_development_only();
        let is_dev_reachable = reachable_dev.contains(&file_id);
        if !is_prod_reachable && !is_dev_reachable {
            continue;
        }

        for dependency in &extracted_file.external_dependencies {
            if is_prod_reachable {
                used_prod_by_workspace
                    .entry(workspace.clone())
                    .or_default()
                    .insert(dependency.clone());
            }
            if is_dev_reachable {
                used_dev_by_workspace
                    .entry(workspace.clone())
                    .or_default()
                    .insert(dependency.clone());
            }
        }
    }

    let mut findings = Vec::new();
    for workspace in build.discovery.workspaces.values() {
        let workspace_name = workspace.name.clone();
        let package_name =
            workspace.manifest.name.clone().unwrap_or_else(|| workspace_name.clone());
        let unresolved_count = workspace_unresolved_specifiers(build, &workspace_name);
        let only_script_entrypoints = workspace_has_only_script_entrypoints(build, &workspace_name);
        let used_prod = used_prod_by_workspace.get(&workspace_name);
        let used_dev = used_dev_by_workspace.get(&workspace_name);

        let manifest_path = workspace
            .root
            .strip_prefix(&build.discovery.project_root)
            .unwrap_or(&workspace.root)
            .join("package.json")
            .to_string_lossy()
            .to_string();

        for (dependency, dependency_kind, used) in
            declared_dependencies(&workspace.manifest, profile, used_prod, used_dev)
        {
            if used {
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
            let confidence = if unresolved_count > 8 || only_script_entrypoints {
                FindingConfidence::Low
            } else {
                FindingConfidence::Medium
            };

            findings.push(make_finding(
                "unused-dependency",
                finding_severity,
                FindingCategory::UnusedDependency,
                confidence,
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

fn workspace_unresolved_specifiers(build: &GraphBuildResult, workspace_name: &str) -> usize {
    build
        .files
        .iter()
        .filter(|file| file.file.workspace.as_deref() == Some(workspace_name))
        .flat_map(|file| file.resolved_imports.iter().chain(&file.resolved_reexports))
        .filter(|edge| matches!(edge.outcome, pruneguard_resolver::ResolutionOutcome::Unresolved))
        .count()
}

fn workspace_has_only_script_entrypoints(build: &GraphBuildResult, workspace_name: &str) -> bool {
    let mut saw_entrypoint = false;
    for seed in &build.entrypoint_seeds {
        if seed.workspace.as_deref() != Some(workspace_name) {
            continue;
        }
        saw_entrypoint = true;
        if seed.kind != pruneguard_entrypoints::EntrypointKind::PackageScript {
            return false;
        }
    }

    saw_entrypoint
}

fn declared_dependencies<'a>(
    manifest: &'a pruneguard_manifest::PackageManifest,
    profile: EntrypointProfile,
    used_prod: Option<&'a FxHashSet<String>>,
    used_dev: Option<&'a FxHashSet<String>>,
) -> Vec<(&'a str, &'static str, bool)> {
    let prod_labels = manifest
        .dependencies
        .iter()
        .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "dependency")));
    let peer_labels = manifest
        .peer_dependencies
        .iter()
        .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "peer dependency")));
    let optional_labels = manifest
        .optional_dependencies
        .iter()
        .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "optional dependency")));
    let dev_labels = manifest
        .dev_dependencies
        .iter()
        .flat_map(|deps| deps.keys().map(|name| (name.as_str(), "dev dependency")));

    let mut dependencies = Vec::new();
    for (dependency, kind) in prod_labels.chain(peer_labels).chain(optional_labels) {
        let used = match profile {
            EntrypointProfile::Production => {
                used_prod.is_some_and(|deps| deps.contains(dependency))
            }
            EntrypointProfile::Development | EntrypointProfile::Both => {
                used_prod.is_some_and(|deps| deps.contains(dependency))
                    || used_dev.is_some_and(|deps| deps.contains(dependency))
            }
        };
        dependencies.push((dependency, kind, used));
    }

    if profile != EntrypointProfile::Production {
        for (dependency, kind) in dev_labels {
            dependencies.push((
                dependency,
                kind,
                used_dev.is_some_and(|deps| deps.contains(dependency)),
            ));
        }
    }

    dependencies.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
    dependencies.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);
    dependencies
}
