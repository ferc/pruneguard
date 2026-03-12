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

            // Skip dependencies that are referenced directly in package.json scripts
            // (e.g. "build": "vite build" means vite is used even without source imports).
            if scripts_reference_dependency(&workspace.manifest, dependency) {
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

/// Check if any package.json script directly references a dependency by name.
///
/// Handles common patterns:
/// - `<pkg> <args>` (binary at start of script value)
/// - `pnpm exec <pkg>`, `npx <pkg>`, `yarn exec <pkg>`
/// - `node_modules/.bin/<pkg>`
fn scripts_reference_dependency(
    manifest: &pruneguard_manifest::PackageManifest,
    dependency: &str,
) -> bool {
    let Some(scripts) = &manifest.scripts else {
        return false;
    };

    // For scoped packages like `@scope/name`, the binary name is typically `name`.
    let bin_name = if let Some(rest) = dependency.strip_prefix('@') {
        rest.split('/').nth(1).unwrap_or(dependency)
    } else {
        dependency
    };

    for script_value in scripts.values() {
        if script_references_bin(script_value, bin_name) {
            return true;
        }
        // Also check the full dependency name for scoped packages used directly.
        if bin_name != dependency && script_references_bin(script_value, dependency) {
            return true;
        }
    }
    false
}

/// Check if a single script command string references a binary name.
fn script_references_bin(script: &str, bin_name: &str) -> bool {
    // Split on common shell operators to handle chained commands.
    for segment in script.split(&['&', '|', ';'][..]) {
        let segment = segment.trim();
        // Handle `node_modules/.bin/<pkg>` anywhere in the segment.
        if segment.contains(&format!("node_modules/.bin/{bin_name}")) {
            return true;
        }

        let mut tokens = segment.split_whitespace();
        let Some(first) = tokens.next() else {
            continue;
        };

        // Direct invocation: `<pkg> build`, `<pkg> --flag`
        if first == bin_name {
            return true;
        }

        // Runner patterns: `pnpm exec <pkg>`, `npx <pkg>`, `yarn exec <pkg>`,
        //                   `pnpm <pkg>`, `yarn <pkg>`, `bunx <pkg>`
        match first {
            "npx" | "bunx" => {
                // npx/bunx may have flags before the package name.
                for token in tokens {
                    if token.starts_with('-') {
                        continue;
                    }
                    return token == bin_name;
                }
            }
            "pnpm" | "yarn" => {
                if let Some(second) = tokens.next() {
                    if second == "exec" || second == "run" || second == "dlx" {
                        // Next non-flag token is the package/binary name.
                        for token in tokens {
                            if token.starts_with('-') {
                                continue;
                            }
                            return token == bin_name;
                        }
                    } else if second == bin_name {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}
