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

    // Scan CSS/SCSS/SASS/LESS files for @import references to npm packages.
    // These files are not parsed by the JS/TS extractor, so any packages loaded
    // exclusively via CSS @import would otherwise be falsely flagged as unused.
    let css_used_by_workspace = collect_css_dependency_references(build);
    for (workspace, deps) in &css_used_by_workspace {
        used_prod_by_workspace.entry(workspace.clone()).or_default().extend(deps.iter().cloned());
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

            // Skip build-tool dependencies that are used by the toolchain rather
            // than imported in source code.
            if is_build_tool_dependency(dependency) {
                continue;
            }

            // Skip framework-implicit runtime dependencies.  For example,
            // `react-dom` is never directly imported in user code but is
            // required at runtime by any React-based framework.
            if is_framework_implicit_dependency(dependency, &workspace.manifest) {
                continue;
            }

            // Skip dependencies that are referenced directly in package.json scripts
            // (e.g. "build": "vite build" means vite is used even without source imports).
            if scripts_reference_dependency(&workspace.manifest, dependency) {
                continue;
            }

            let benign_unresolved = workspace_benign_unresolved_specifiers(build, &workspace_name);
            let effective_unresolved = unresolved_count.saturating_sub(benign_unresolved);

            let evidence = vec![Evidence {
                kind: "dependency".to_string(),
                file: Some(manifest_path.clone()),
                line: None,
                description: format!(
                    "No reachable file in the active profile resolved to this {dependency_kind}."
                ),
            }];
            // unused-dependency defaults to Medium; Low when unresolved-heavy.
            let confidence = if effective_unresolved > 8
                || only_script_entrypoints
                || (effective_unresolved > 3 && dependency_kind == "peer dependency")
            {
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

fn workspace_benign_unresolved_specifiers(build: &GraphBuildResult, workspace_name: &str) -> usize {
    build
        .files
        .iter()
        .filter(|file| file.file.workspace.as_deref() == Some(workspace_name))
        .flat_map(|file| file.resolved_imports.iter().chain(&file.resolved_reexports))
        .filter(|edge| {
            matches!(edge.outcome, pruneguard_resolver::ResolutionOutcome::Unresolved)
                && edge
                    .unresolved_reason
                    .is_some_and(pruneguard_resolver::UnresolvedReason::is_benign)
        })
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
            // A devDependency is "used" if it is imported from either
            // development-profile OR production-profile reachable files.
            // A devDep imported in production code is misclassified (should
            // be a regular dependency) but it is certainly not unused.
            dependencies.push((
                dependency,
                kind,
                used_dev.is_some_and(|deps| deps.contains(dependency))
                    || used_prod.is_some_and(|deps| deps.contains(dependency)),
            ));
        }
    }

    dependencies.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
    dependencies.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);
    dependencies
}

/// Dependencies that are consumed by the build toolchain or runtime environment
/// rather than imported in source code.  These will never resolve through the
/// module graph, so flagging them as unused is a false positive.
#[allow(clippy::too_many_lines)]
fn is_build_tool_dependency(dep: &str) -> bool {
    // @types/* packages are consumed by the TypeScript compiler, not imported.
    if dep.starts_with("@types/") {
        return true;
    }

    // ESLint plugins and configs are loaded by the ESLint runner, not imported.
    if dep.starts_with("@eslint/")
        || dep.starts_with("eslint-plugin-")
        || dep.starts_with("eslint-config-")
        || dep.starts_with("@next/eslint-plugin-")
        || dep.starts_with("@typescript-eslint/")
    {
        return true;
    }

    // Prettier plugins are loaded by the Prettier runner.
    if dep.starts_with("prettier-plugin-") {
        return true;
    }

    // Storybook addons and builders are loaded by .storybook/main config.
    if dep.starts_with("@storybook/") {
        return true;
    }

    // OpenTelemetry instrumentation packages are loaded by runtime config.
    if dep.starts_with("@opentelemetry/") {
        return true;
    }

    // Babel plugins and presets.
    if dep.starts_with("babel-plugin-")
        || dep.starts_with("@babel/plugin-")
        || dep.starts_with("@babel/preset-")
    {
        return true;
    }

    // PostCSS plugins.
    if dep.starts_with("postcss-") {
        return true;
    }

    // Rollup plugins.
    if dep.starts_with("rollup-plugin-") || dep.starts_with("@rollup/plugin-") {
        return true;
    }

    // Webpack loaders and plugins.
    if dep.ends_with("-loader") || dep.starts_with("webpack-") {
        return true;
    }

    // Stylelint plugins and configs.
    if dep.starts_with("stylelint-") {
        return true;
    }

    // Commitlint configs and plugins.
    if dep.starts_with("@commitlint/") || dep.starts_with("commitlint-") {
        return true;
    }

    // Semantic-release plugins.
    if dep.starts_with("@semantic-release/") || dep.starts_with("semantic-release-") {
        return true;
    }

    matches!(
        dep,
        // Language / compiler
        "typescript"
            // CSS toolchain
            | "postcss"
            | "autoprefixer"
            | "tailwindcss"
            | "@tailwindcss/typography"
            | "@tailwindcss/forms"
            | "@tailwindcss/container-queries"
            | "@tailwindcss/vite"
            | "@tailwindcss/postcss"
            | "cssnano"
            | "sass"
            | "less"
            | "lightningcss"
            // Bundler plugins (loaded by config, not imported)
            | "@vitejs/plugin-react"
            | "@vitejs/plugin-react-swc"
            | "@vitejs/plugin-vue"
            | "vite-tsconfig-paths"
            | "vite-plugin-dts"
            | "@tanstack/router-plugin"
            | "@tanstack/router-vite-plugin"
            | "@content-collections/core"
            | "@content-collections/vite"
            | "@content-collections/next"
            | "@next/bundle-analyzer"
            | "@sentry/esbuild-plugin"
            | "@sentry/nextjs"
            | "esbuild"
            | "swc"
            | "@swc/core"
            | "@swc/cli"
            | "terser"
            | "rollup"
            | "webpack"
            | "webpack-cli"
            | "webpack-dev-server"
            // Test runners & frameworks (invoked by config, not imported in source)
            | "@playwright/test"
            | "playwright"
            | "cypress"
            | "@cypress/code-coverage"
            | "c8"
            | "nyc"
            | "istanbul"
            // Linting / formatting (invoked by scripts or config, not imported)
            | "eslint"
            | "prettier"
            | "stylelint"
            | "oxlint"
            | "biome"
            | "@biomejs/biome"
            | "lint-staged"
            | "husky"
            | "simple-git-hooks"
            | "lefthook"
            // Script runners (invoked via scripts, not imported)
            | "tsx"
            | "ts-node"
            | "tsm"
            | "nodemon"
            | "concurrently"
            | "wait-on"
            | "npm-run-all"
            | "npm-run-all2"
            | "cross-env"
            | "dotenv-cli"
            | "env-cmd"
            // Documentation tools
            | "typedoc"
            | "jsdoc"
            // Release / versioning tools
            | "changesets"
            | "@changesets/cli"
            | "@changesets/changelog-github"
            | "standard-version"
            | "release-it"
            | "np"
            | "semantic-release"
    )
}

/// Dependencies that are implicitly required at runtime when a framework or
/// library is present.  These are never directly imported in user code but
/// must exist in `node_modules` for the framework to function.
fn is_framework_implicit_dependency(
    dep: &str,
    manifest: &pruneguard_manifest::PackageManifest,
) -> bool {
    let has_dep = |name: &str| -> bool {
        manifest.dependencies.as_ref().is_some_and(|deps| deps.contains_key(name))
            || manifest.dev_dependencies.as_ref().is_some_and(|deps| deps.contains_key(name))
            || manifest.peer_dependencies.as_ref().is_some_and(|deps| deps.contains_key(name))
    };

    match dep {
        // react-dom is the DOM renderer for React — required by every React
        // framework (Next.js, Vite+React, TanStack Start, Remix, CRA, etc.)
        // but never directly imported in user code with modern JSX transforms.
        "react-dom" => has_dep("react") || has_dep("next") || has_dep("@tanstack/react-start"),
        _ => false,
    }
}

/// Scan CSS/SCSS/SASS/LESS files in the project for `@import` statements that
/// reference npm packages.  Returns a map of workspace name → set of used
/// package names discovered in stylesheets.
fn collect_css_dependency_references(
    build: &GraphBuildResult,
) -> FxHashMap<String, FxHashSet<String>> {
    let mut result: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    let css_extensions = ["css", "scss", "sass", "less", "styl"];

    for workspace in build.discovery.workspaces.values() {
        // Walk CSS files using the ignore crate to respect .gitignore.
        let mut walk_builder = ignore::WalkBuilder::new(&workspace.root);
        walk_builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);

        for entry in walk_builder.build().flatten() {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let path = entry.into_path();
            let is_css = path
                .extension()
                .and_then(|ext: &std::ffi::OsStr| ext.to_str())
                .is_some_and(|ext| css_extensions.contains(&ext));
            if !is_css {
                continue;
            }
            // Skip node_modules and build output directories.
            let path_str = path.to_string_lossy();
            if path_str.contains("node_modules") || path_str.contains("/dist/") {
                continue;
            }

            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            for dep in extract_css_import_packages(&content) {
                result.entry(workspace.name.clone()).or_default().insert(dep);
            }
        }
    }

    result
}

/// Extract npm package names from CSS `@import` statements.
///
/// Handles:
/// - `@import "package-name";`
/// - `@import 'package-name';`
/// - `@import "package-name/subpath";`
/// - `@import "@scope/package-name";`
/// - `@import url("package-name");`
fn extract_css_import_packages(source: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("@import") && !trimmed.starts_with("@use") {
            continue;
        }

        // Extract the value between quotes.
        let value = trimmed.split('"').nth(1).or_else(|| trimmed.split('\'').nth(1));
        let Some(value) = value else {
            continue;
        };

        // Skip relative imports and URLs.
        if value.starts_with('.') || value.starts_with('/') || value.starts_with("http") {
            continue;
        }
        // Strip url() wrapper if present.
        let value = value.strip_prefix("url(").unwrap_or(value);
        let value = value.strip_suffix(')').unwrap_or(value);

        // Extract the package name (handle scoped packages).
        if let Some(pkg) = css_import_to_package_name(value) {
            packages.push(pkg);
        }
    }
    packages
}

/// Map a CSS import specifier to an npm package name.
fn css_import_to_package_name(specifier: &str) -> Option<String> {
    if specifier.is_empty() || specifier.starts_with('.') || specifier.starts_with('/') {
        return None;
    }
    let mut parts = specifier.split('/');
    let first = parts.next()?;
    if first.starts_with('@') {
        let second = parts.next()?;
        Some(format!("{first}/{second}"))
    } else {
        Some(first.to_string())
    }
}

/// Check if any package.json script directly references a dependency by name.
///
/// Handles common patterns:
/// - `<pkg> <args>` (binary at start of script value)
/// - `pnpm exec <pkg>`, `npx <pkg>`, `yarn exec <pkg>`
/// - `node_modules/.bin/<pkg>`
/// - `node -r <pkg>`, `node --require <pkg>`
/// - `pnpm --filter <ws> run <script>` (workspace-invoked scripts)
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
        //                   `pnpm <pkg>`, `yarn <pkg>`, `bunx <pkg>`,
        //                   `tsx <file>`, `ts-node <file>`, `npm exec <pkg>`
        match first {
            "npx" | "bunx" | "tsx" | "ts-node" | "tsm" => {
                // npx/bunx/tsx/ts-node may have flags before the package name.
                for token in tokens {
                    if token.starts_with('-') {
                        continue;
                    }
                    return token == bin_name;
                }
            }
            "pnpm" | "yarn" | "npm" | "bun" => {
                // Consume flags like --filter, --workspace, -w, etc.
                let mut subcommand = None;
                while let Some(token) = tokens.next() {
                    if token.starts_with('-') {
                        // Some flags take a value (e.g. `--filter <ws>`).
                        if matches!(
                            token,
                            "--filter" | "-F" | "--workspace" | "-w" | "--cwd" | "--prefix"
                        ) {
                            let _ = tokens.next(); // consume the flag value
                        }
                        continue;
                    }
                    subcommand = Some(token);
                    break;
                }
                if let Some(sub) = subcommand {
                    if sub == "exec" || sub == "run" || sub == "dlx" || sub == "x" {
                        // Next non-flag token is the package/binary name.
                        for token in tokens {
                            if token.starts_with('-') {
                                continue;
                            }
                            return token == bin_name;
                        }
                    } else if sub == bin_name {
                        return true;
                    }
                }
            }
            "node" => {
                // Handle `node -r <pkg>`, `node --require <pkg>`,
                // `node --loader <pkg>`, `node --import <pkg>`.
                while let Some(token) = tokens.next() {
                    if matches!(token, "-r" | "--require" | "--loader" | "--import") {
                        if let Some(pkg) = tokens.next() {
                            // The required module might be the dependency itself
                            // or a subpath like `pkg/register`.
                            let required_pkg = pkg.split('/').next().unwrap_or(pkg);
                            if required_pkg == bin_name {
                                return true;
                            }
                        }
                    } else if token.starts_with("--require=") || token.starts_with("-r=") {
                        let value = token.split('=').nth(1).unwrap_or("");
                        let required_pkg = value.split('/').next().unwrap_or(value);
                        if required_pkg == bin_name {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}
