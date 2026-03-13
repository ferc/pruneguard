use std::collections::BTreeMap;

use globset::{Glob, GlobSet, GlobSetBuilder};
use petgraph::visit::EdgeRef;
use rustc_hash::{FxHashMap, FxHashSet};

use pruneguard_config::{OwnershipConfig, PruneguardConfig};
use pruneguard_graph::{GraphBuildResult, ModuleEdge, ModuleNode};
use pruneguard_report::{
    EffortLevel, FindingConfidence, GovernanceAction, GovernanceActionKind, Hotspot, ImpactLevel,
    OwnershipHint, SuggestRulesReport, SuggestedRule, SuggestedRuleKind, SuggestedTag,
};

/// Threshold: minimum cross-package edge count before suggesting a boundary rule.
const BOUNDARY_EDGE_THRESHOLD: usize = 3;

/// Threshold: minimum files in a directory pattern to suggest a tag.
const TAG_FILE_THRESHOLD: usize = 3;

/// Threshold: minimum total edges to flag a file as a hotspot.
const HOTSPOT_EDGE_THRESHOLD: usize = 5;

/// Threshold: minimum distinct packages in a cluster before suggesting layering.
const LAYER_CLUSTER_THRESHOLD: usize = 3;

/// Maximum number of hotspots to report.
const MAX_HOTSPOTS: usize = 20;

/// Threshold: minimum bidirectional edge count to flag mutual coupling.
const MUTUAL_COUPLING_THRESHOLD: usize = 2;

/// Analyze the graph and configuration to suggest governance rules.
pub fn suggest_rules(graph: &GraphBuildResult, config: &PruneguardConfig) -> SuggestRulesReport {
    let mut report = SuggestRulesReport::default();

    // Compute ownership map once for reuse across multiple suggestion passes.
    let ownership_map = build_ownership_map(graph, config.ownership.as_ref());

    suggest_boundary_rules(graph, config, &mut report);
    suggest_mutual_coupling_rules(graph, config, &mut report);
    suggest_tags_from_directories(graph, &mut report);
    suggest_tags_from_structure(graph, config, &mut report);
    suggest_tags_from_frameworks(graph, &mut report);
    suggest_tag_assignment_rules(&mut report);
    suggest_reachability_fences(graph, config, &mut report);
    suggest_layer_enforcement(graph, &mut report);
    suggest_hotspots(graph, &ownership_map, &mut report);
    suggest_ownership_hints(graph, config, &ownership_map, &mut report);
    suggest_ownership_from_codeowners(graph, &mut report);
    synthesize_governance_actions(&mut report);

    if report.suggested_rules.is_empty()
        && report.tags.is_empty()
        && report.hotspots.is_empty()
        && report.ownership_hints.is_empty()
    {
        report.rationale.push(
            "No governance suggestions were generated. The project may be too small or \
             already well-configured."
                .to_string(),
        );
    }

    report
}

// ---------------------------------------------------------------------------
// Ownership map: file relative path -> owner name
// ---------------------------------------------------------------------------

fn build_ownership_map(
    graph: &GraphBuildResult,
    ownership: Option<&OwnershipConfig>,
) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();

    let team_matchers = ownership
        .and_then(|config| config.teams.as_ref())
        .map(|teams| {
            teams
                .iter()
                .map(|(team, config)| OwnerMatcher {
                    team: team.clone(),
                    path_matcher: compile_globset(&config.paths),
                    packages: config.packages.iter().cloned().collect(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for extracted_file in &graph.files {
        let relative_path = extracted_file.file.relative_path.to_string_lossy();

        // Try team matchers first.
        if let Some(owner) = team_matchers.iter().find_map(|matcher| {
            let path_match = matcher
                .path_matcher
                .as_ref()
                .is_some_and(|glob| glob.is_match(relative_path.as_ref()));
            let pkg_match = extracted_file
                .file
                .package
                .as_ref()
                .is_some_and(|pkg| matcher.packages.contains(pkg));
            if path_match || pkg_match { Some(matcher.team.clone()) } else { None }
        }) {
            map.insert(relative_path.to_string(), owner);
            continue;
        }

        // Try CODEOWNERS.
        if let Some(codeowners) = &graph.discovery.codeowners
            && let Some(owners) = match_codeowners(codeowners, &relative_path)
        {
            map.insert(relative_path.to_string(), owners);
        }
    }

    map
}

struct OwnerMatcher {
    team: String,
    path_matcher: Option<GlobSet>,
    packages: FxHashSet<String>,
}

fn match_codeowners(
    codeowners: &pruneguard_discovery::Codeowners,
    relative_path: &str,
) -> Option<String> {
    let mut matched = None;
    for rule in &codeowners.rules {
        if codeowners_pattern_matches(&rule.pattern, relative_path) {
            matched = Some(rule.owners.join(" "));
        }
    }
    matched
}

fn codeowners_pattern_matches(pattern: &str, relative_path: &str) -> bool {
    let normalized = pattern.trim_start_matches('/');
    let glob = if normalized.ends_with('/') {
        format!("{normalized}**")
    } else if normalized.contains('*') {
        normalized.to_string()
    } else {
        format!("{normalized}**")
    };

    Glob::new(&glob).ok().is_some_and(|compiled| compiled.compile_matcher().is_match(relative_path))
}

// ---------------------------------------------------------------------------
// 1. Boundary rules from cross-package traffic clusters
// ---------------------------------------------------------------------------

/// Track the coupling strength between two packages to distinguish
/// type-only coupling from runtime value coupling.
#[derive(Default)]
struct CrossPackageCouplingMetrics {
    total_edges: usize,
    type_only_edges: usize,
    value_edges: usize,
    dynamic_edges: usize,
    distinct_source_files: FxHashSet<String>,
}

/// Suggest forbidden-dependency boundary rules by analyzing cross-package import edges.
fn suggest_boundary_rules(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    if config.rules.is_some() {
        report
            .rationale
            .push("Boundary rules already configured; skipping boundary suggestions.".to_string());
        return;
    }

    let coupling = collect_cross_package_coupling(graph);

    emit_boundary_rules_from_coupling(&coupling, report);

    if !coupling.is_empty() && report.suggested_rules.is_empty() {
        report.rationale.push(
            "Cross-package imports exist but none exceed the threshold for a boundary suggestion."
                .to_string(),
        );
    }
}

fn collect_cross_package_coupling(
    graph: &GraphBuildResult,
) -> BTreeMap<(String, String), CrossPackageCouplingMetrics> {
    let mut coupling: BTreeMap<(String, String), CrossPackageCouplingMetrics> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File { package: Some(source_package), relative_path, .. } =
            &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }

            let target = edge.target();
            if let ModuleNode::File { package: Some(target_package), .. } =
                &graph.module_graph.graph[target]
                && source_package != target_package
            {
                let key = (source_package.clone(), target_package.clone());
                let metrics = coupling.entry(key).or_default();
                metrics.total_edges += 1;
                metrics.distinct_source_files.insert(relative_path.clone());

                match edge.weight() {
                    ModuleEdge::StaticImportType => metrics.type_only_edges += 1,
                    ModuleEdge::DynamicImport => metrics.dynamic_edges += 1,
                    _ => metrics.value_edges += 1,
                }
            }
        }
    }

    coupling
}

fn emit_boundary_rules_from_coupling(
    coupling: &BTreeMap<(String, String), CrossPackageCouplingMetrics>,
    report: &mut SuggestRulesReport,
) {
    for ((source, target), metrics) in coupling {
        if metrics.total_edges < BOUNDARY_EDGE_THRESHOLD {
            continue;
        }

        let distinct_files = metrics.distinct_source_files.len();
        let name = format!("forbid-{source}-to-{target}");
        let confidence = if metrics.total_edges >= BOUNDARY_EDGE_THRESHOLD * 3 {
            FindingConfidence::High
        } else if metrics.total_edges >= BOUNDARY_EDGE_THRESHOLD * 2 {
            FindingConfidence::Medium
        } else {
            FindingConfidence::Low
        };

        let is_type_only = metrics.type_only_edges == metrics.total_edges;

        let config_fragment = if is_type_only {
            serde_json::json!({
                "rules": {
                    "forbidden": [{
                        "name": name,
                        "severity": "warn",
                        "comment": format!("Type-only coupling from `{source}` to `{target}` -- consider if this should be runtime-enforced."),
                        "from": { "package": [source] },
                        "to": { "package": [target], "dependencyKinds": ["static-value", "dynamic", "require"] }
                    }]
                }
            })
        } else {
            serde_json::json!({
                "rules": {
                    "forbidden": [{
                        "name": name,
                        "severity": "error",
                        "from": { "package": [source] },
                        "to": { "package": [target] }
                    }]
                }
            })
        };

        let coupling_breakdown = format!(
            "{} value, {} type-only, {} dynamic",
            metrics.value_edges, metrics.type_only_edges, metrics.dynamic_edges
        );

        let rationale = format!(
            "Package `{source}` imports from `{target}` across {} edges \
             from {distinct_files} distinct source file(s) ({coupling_breakdown}). \
             {type_note}\
             A high edge count between packages is a strong signal of a coupling boundary \
             that should be made explicit. Adding a forbidden rule lets you enforce this \
             boundary in CI and catch regressions.",
            metrics.total_edges,
            type_note = if is_type_only {
                "All edges are type-only imports, so a less strict rule is suggested. "
            } else {
                ""
            }
        );

        report.suggested_rules.push(SuggestedRule {
            kind: SuggestedRuleKind::Forbidden,
            name,
            description: format!(
                "Package `{source}` imports from `{target}` across {} edges ({coupling_breakdown}). \
                 Consider a forbidden rule if this crosses an intended boundary.",
                metrics.total_edges
            ),
            config_fragment,
            confidence,
            evidence: vec![
                format!("{} cross-package edges from `{source}` to `{target}`", metrics.total_edges),
                format!("{distinct_files} distinct source files participate in the coupling"),
                format!("Coupling breakdown: {coupling_breakdown}"),
            ],
            rationale: Some(rationale),
        });
    }
}

// ---------------------------------------------------------------------------
// 1b. Mutual coupling detection (bidirectional dependencies)
// ---------------------------------------------------------------------------

/// Detect packages that mutually depend on each other (A imports B and B imports A).
fn suggest_mutual_coupling_rules(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    if config.rules.is_some() {
        return;
    }

    // Count edges in both directions between package pairs.
    let mut edge_counts: BTreeMap<(String, String), usize> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File { package: Some(source_package), .. } =
            &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            if let ModuleNode::File { package: Some(target_package), .. } =
                &graph.module_graph.graph[edge.target()]
                && source_package != target_package
            {
                *edge_counts
                    .entry((source_package.clone(), target_package.clone()))
                    .or_insert(0) += 1;
            }
        }
    }

    // Find mutual pairs where both directions exceed the threshold.
    let mut seen_pairs = FxHashSet::<(String, String)>::default();
    for ((source, target), forward_count) in &edge_counts {
        if *forward_count < MUTUAL_COUPLING_THRESHOLD {
            continue;
        }

        let reverse_key = (target.clone(), source.clone());
        let reverse_count = edge_counts.get(&reverse_key).copied().unwrap_or(0);
        if reverse_count < MUTUAL_COUPLING_THRESHOLD {
            continue;
        }

        // Normalize the pair to avoid duplicates.
        let normalized = if source < target {
            (source.clone(), target.clone())
        } else {
            (target.clone(), source.clone())
        };
        if !seen_pairs.insert(normalized.clone()) {
            continue;
        }

        let (pkg_a, pkg_b) = &normalized;
        let a_to_b = edge_counts.get(&(pkg_a.clone(), pkg_b.clone())).copied().unwrap_or(0);
        let b_to_a = edge_counts.get(&(pkg_b.clone(), pkg_a.clone())).copied().unwrap_or(0);
        let total = a_to_b + b_to_a;

        let confidence = if total >= BOUNDARY_EDGE_THRESHOLD * 4 {
            FindingConfidence::High
        } else if total >= BOUNDARY_EDGE_THRESHOLD * 2 {
            FindingConfidence::Medium
        } else {
            FindingConfidence::Low
        };

        let name = format!("decouple-{pkg_a}-and-{pkg_b}");
        let config_fragment = serde_json::json!({
            "rules": {
                "forbidden": [
                    {
                        "name": format!("forbid-{pkg_a}-to-{pkg_b}"),
                        "severity": "error",
                        "comment": format!("Break mutual coupling: `{pkg_a}` should not import `{pkg_b}`."),
                        "from": { "package": [pkg_a] },
                        "to": { "package": [pkg_b] }
                    },
                    {
                        "name": format!("forbid-{pkg_b}-to-{pkg_a}"),
                        "severity": "error",
                        "comment": format!("Break mutual coupling: `{pkg_b}` should not import `{pkg_a}`."),
                        "from": { "package": [pkg_b] },
                        "to": { "package": [pkg_a] }
                    }
                ]
            }
        });

        report.suggested_rules.push(SuggestedRule {
            kind: SuggestedRuleKind::Forbidden,
            name,
            description: format!(
                "Packages `{pkg_a}` and `{pkg_b}` have mutual dependencies \
                 ({a_to_b} edges A->B, {b_to_a} edges B->A). \
                 Mutual coupling creates implicit cycles that block independent deployment."
            ),
            config_fragment,
            confidence,
            evidence: vec![
                format!("{a_to_b} edges from `{pkg_a}` to `{pkg_b}`"),
                format!("{b_to_a} edges from `{pkg_b}` to `{pkg_a}`"),
                format!(
                    "Mutual coupling total: {total} edges. \
                     Extract shared code into a third package or invert one direction."
                ),
            ],
            rationale: Some(format!(
                "Bidirectional dependencies between `{pkg_a}` and `{pkg_b}` create an implicit \
                 cycle at the package level. This makes both packages effectively one unit: \
                 changing either requires rebuilding and testing both. Breaking the cycle \
                 restores independent deployability and clearer ownership boundaries."
            )),
        });
    }
}

// ---------------------------------------------------------------------------
// 2a. Tags from directory clustering
// ---------------------------------------------------------------------------

/// Suggest tags by grouping files into common directory patterns.
fn suggest_tags_from_directories(graph: &GraphBuildResult, report: &mut SuggestRulesReport) {
    // Group files by their first two path segments (e.g. "src/components").
    let mut dir_counts: BTreeMap<String, usize> = BTreeMap::new();

    for file in &graph.files {
        let relative = file.file.relative_path.to_string_lossy();
        if let Some(dir) = extract_tag_directory(&relative) {
            *dir_counts.entry(dir).or_insert(0) += 1;
        }
    }

    for (dir, count) in &dir_counts {
        if *count < TAG_FILE_THRESHOLD {
            continue;
        }

        let tag_name = dir.replace(['/', '\\'], "-");
        let glob = format!("{dir}/**");
        let rationale = format!(
            "{count} files match the pattern `{glob}`. A tag groups these files \
             so you can write boundary rules targeting them by tag name instead of \
             brittle path globs. This makes rules resilient to directory renames."
        );

        report.tags.push(SuggestedTag {
            name: tag_name,
            glob,
            source: Some("directory-cluster".to_string()),
            rationale,
        });
    }
}

// ---------------------------------------------------------------------------
// 2b. Tags from package/workspace/ownership structure
// ---------------------------------------------------------------------------

/// Suggest tags derived from workspace names, package names, and team ownership.
fn suggest_tags_from_structure(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    // Skip if overrides already assign tags -- the user is already using the system.
    let has_override_tags = config.overrides.iter().any(|o| !o.tags.is_empty());
    let has_team_tags = config
        .ownership
        .as_ref()
        .and_then(|o| o.teams.as_ref())
        .is_some_and(|teams| teams.values().any(|t| !t.tags.is_empty()));

    if has_override_tags && has_team_tags {
        report.rationale.push(
            "Tags are already assigned via overrides and ownership; skipping structural tag suggestions."
                .to_string(),
        );
        return;
    }

    // Suggest workspace-based tags.
    if !has_override_tags {
        let mut workspace_file_counts: BTreeMap<String, usize> = BTreeMap::new();
        for file in &graph.files {
            if let Some(workspace) = &file.file.workspace {
                *workspace_file_counts.entry(workspace.clone()).or_insert(0) += 1;
            }
        }

        for (workspace, count) in &workspace_file_counts {
            if *count < TAG_FILE_THRESHOLD {
                continue;
            }

            let tag_name = format!("ws-{}", workspace.replace(['/', '\\', '@'], "-"));
            report.tags.push(SuggestedTag {
                name: tag_name,
                glob: workspace.clone(),
                source: Some("workspace".to_string()),
                rationale: format!(
                    "Workspace `{workspace}` contains {count} files. A workspace-level tag \
                     enables rules like \"nothing outside this workspace may import its internals\" \
                     without listing individual paths."
                ),
            });
        }
    }

    // Suggest package-based tags for packages with significant file counts.
    let mut package_file_counts: BTreeMap<String, usize> = BTreeMap::new();
    for file in &graph.files {
        if let Some(package) = &file.file.package {
            *package_file_counts.entry(package.clone()).or_insert(0) += 1;
        }
    }

    for (package, count) in &package_file_counts {
        if *count < TAG_FILE_THRESHOLD {
            continue;
        }

        let tag_name = format!("pkg-{}", package.replace(['/', '\\', '@', '.'], "-"));
        report.tags.push(SuggestedTag {
            name: tag_name,
            glob: package.clone(),
            source: Some("package".to_string()),
            rationale: format!(
                "Package `{package}` contains {count} files. A package-level tag allows \
                 tag-based boundary rules (e.g. forbid `tag:pkg-X` from importing `tag:pkg-Y`)."
            ),
        });
    }

    // Suggest ownership-based tags from team config if teams exist but have no tags.
    if !has_team_tags && let Some(teams) = config.ownership.as_ref().and_then(|o| o.teams.as_ref())
    {
        for (team_name, team_config) in teams {
            if !team_config.tags.is_empty() {
                continue;
            }
            let tag_name = format!("team-{team_name}");
            let paths_desc = if team_config.paths.is_empty() {
                String::new()
            } else {
                format!(" (paths: {})", team_config.paths.join(", "))
            };
            let pkgs_desc = if team_config.packages.is_empty() {
                String::new()
            } else {
                format!(" (packages: {})", team_config.packages.join(", "))
            };
            report.tags.push(SuggestedTag {
                name: tag_name.clone(),
                glob: team_config
                    .paths
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("<{team_name}-paths>")),
                source: Some("ownership".to_string()),
                rationale: format!(
                    "Team `{team_name}` is defined in ownership config{paths_desc}{pkgs_desc} \
                         but has no tags. Adding a `{tag_name}` tag enables tag-based boundary \
                         rules that automatically apply to all files owned by this team."
                ),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 2c. Tags from framework detection
// ---------------------------------------------------------------------------

/// Known framework signatures: (dependency name, tag name, description).
const FRAMEWORK_SIGNATURES: &[(&str, &str, &str)] = &[
    ("react", "framework-react", "React framework detected"),
    ("react-dom", "framework-react", "React DOM detected"),
    ("next", "framework-next", "Next.js framework detected"),
    ("vue", "framework-vue", "Vue.js framework detected"),
    ("nuxt", "framework-nuxt", "Nuxt.js framework detected"),
    ("@angular/core", "framework-angular", "Angular framework detected"),
    ("svelte", "framework-svelte", "Svelte framework detected"),
    ("@sveltejs/kit", "framework-sveltekit", "SvelteKit framework detected"),
    ("solid-js", "framework-solid", "SolidJS framework detected"),
    ("@remix-run/react", "framework-remix", "Remix framework detected"),
    ("astro", "framework-astro", "Astro framework detected"),
    ("express", "framework-express", "Express.js framework detected"),
    ("fastify", "framework-fastify", "Fastify framework detected"),
    ("@nestjs/core", "framework-nest", "NestJS framework detected"),
    ("hono", "framework-hono", "Hono framework detected"),
    ("@trpc/server", "framework-trpc", "tRPC framework detected"),
    ("graphql", "framework-graphql", "GraphQL detected"),
    ("@apollo/server", "framework-apollo", "Apollo Server detected"),
    ("prisma", "framework-prisma", "Prisma ORM detected"),
    ("drizzle-orm", "framework-drizzle", "Drizzle ORM detected"),
    ("tailwindcss", "framework-tailwind", "Tailwind CSS detected"),
    ("@storybook/react", "framework-storybook", "Storybook detected"),
    ("@storybook/vue3", "framework-storybook", "Storybook detected"),
    ("vitest", "framework-vitest", "Vitest test framework detected"),
    ("jest", "framework-jest", "Jest test framework detected"),
    ("playwright", "framework-playwright", "Playwright E2E testing detected"),
    ("cypress", "framework-cypress", "Cypress E2E testing detected"),
    ("electron", "framework-electron", "Electron framework detected"),
    ("react-native", "framework-react-native", "React Native framework detected"),
    ("expo", "framework-expo", "Expo framework detected"),
];

/// File pattern signatures: (glob pattern, tag name, description).
const FILE_PATTERN_SIGNATURES: &[(&str, &str, &str)] = &[
    ("**/pages/**", "role-pages", "Pages directory (routing layer)"),
    ("**/app/**", "role-app", "App directory (routing or app shell)"),
    ("**/api/**", "role-api", "API directory (backend endpoints)"),
    ("**/components/**", "role-components", "Components directory (UI layer)"),
    ("**/hooks/**", "role-hooks", "Hooks directory (shared logic)"),
    ("**/utils/**", "role-utils", "Utils directory (shared utilities)"),
    ("**/lib/**", "role-lib", "Lib directory (shared library code)"),
    ("**/services/**", "role-services", "Services directory (business logic)"),
    ("**/models/**", "role-models", "Models directory (data layer)"),
    ("**/types/**", "role-types", "Types directory (type definitions)"),
    ("**/middleware/**", "role-middleware", "Middleware directory"),
    ("**/store/**", "role-store", "Store directory (state management)"),
    ("**/stores/**", "role-store", "Stores directory (state management)"),
    ("**/config/**", "role-config", "Config directory"),
    ("**/constants/**", "role-constants", "Constants directory"),
    ("**/helpers/**", "role-helpers", "Helpers directory (utility layer)"),
];

/// Suggest tags based on framework detection from package dependencies and file patterns.
fn suggest_tags_from_frameworks(graph: &GraphBuildResult, report: &mut SuggestRulesReport) {
    // Collect all dependencies across all workspaces.
    let mut all_deps = FxHashSet::<String>::default();
    for workspace in graph.discovery.workspaces.values() {
        if let Some(deps) = &workspace.manifest.dependencies {
            all_deps.extend(deps.keys().cloned());
        }
        if let Some(deps) = &workspace.manifest.dev_dependencies {
            all_deps.extend(deps.keys().cloned());
        }
        if let Some(deps) = &workspace.manifest.peer_dependencies {
            all_deps.extend(deps.keys().cloned());
        }
    }

    // Check framework dependency signatures.
    let mut detected_frameworks = FxHashSet::<String>::default();
    for &(dep_name, tag_name, description) in FRAMEWORK_SIGNATURES {
        if all_deps.contains(dep_name) && detected_frameworks.insert(tag_name.to_string()) {
            report.tags.push(SuggestedTag {
                name: tag_name.to_string(),
                glob: "**/*".to_string(),
                source: Some("framework-detection".to_string()),
                rationale: format!(
                    "{description} via dependency `{dep_name}`. Framework-aware tags enable \
                     rules specific to this framework's conventions (e.g. restricting server-only \
                     code from client components)."
                ),
            });
        }
    }

    // Check file pattern signatures: only suggest if the pattern matches enough files.
    let mut pattern_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for file in &graph.files {
        let relative = file.file.relative_path.to_string_lossy();
        for &(_, tag_name, _) in FILE_PATTERN_SIGNATURES {
            let parts: Vec<&str> = relative.split('/').collect();
            // Check if any path segment matches the directory name from the pattern.
            let dir_name = tag_name.strip_prefix("role-").unwrap_or(tag_name);
            if parts.iter().any(|part| part.eq_ignore_ascii_case(dir_name)) {
                *pattern_counts.entry(tag_name).or_insert(0) += 1;
            }
        }
    }

    for &(pattern, tag_name, description) in FILE_PATTERN_SIGNATURES {
        let count = pattern_counts.get(tag_name).copied().unwrap_or(0);
        if count < TAG_FILE_THRESHOLD {
            continue;
        }
        // Avoid duplicating tags we already suggested from directory clustering.
        if report.tags.iter().any(|t| t.name == tag_name) {
            continue;
        }

        report.tags.push(SuggestedTag {
            name: tag_name.to_string(),
            glob: pattern.to_string(),
            source: Some("file-pattern".to_string()),
            rationale: format!(
                "{description}: {count} files match `{pattern}`. \
                 Role-based tags let you express architectural constraints like \
                 \"components should not import from services directly.\""
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// 2d. Tag assignment rules: concrete config fragments for suggested tags
// ---------------------------------------------------------------------------

/// Generate `TagAssignment` config fragments for the suggested tags.
fn suggest_tag_assignment_rules(report: &mut SuggestRulesReport) {
    // Only generate tag assignment rules for tags that have concrete globs
    // (not framework-level tags that apply to everything).
    let assignable_tags: Vec<(String, String, String)> = report
        .tags
        .iter()
        .filter(|tag| tag.source.as_deref() != Some("framework-detection") && tag.glob != "**/*")
        .map(|tag| (tag.name.clone(), tag.glob.clone(), tag.source.clone().unwrap_or_default()))
        .collect();

    if assignable_tags.is_empty() {
        return;
    }

    // Group tags by source type for better config organization.
    let mut overrides: Vec<serde_json::Value> = Vec::new();
    for (tag_name, glob, source) in &assignable_tags {
        let override_entry = match source.as_str() {
            "workspace" => serde_json::json!({
                "workspaces": [glob],
                "tags": [tag_name]
            }),
            _ => serde_json::json!({
                "files": [glob],
                "tags": [tag_name]
            }),
        };
        overrides.push(override_entry);
    }

    let config_fragment = serde_json::json!({
        "overrides": overrides
    });

    let tag_count = assignable_tags.len();
    report.suggested_rules.push(SuggestedRule {
        kind: SuggestedRuleKind::TagAssignment,
        name: "assign-suggested-tags".to_string(),
        description: format!(
            "Assign {tag_count} suggested tag(s) via overrides configuration. \
             Tags enable tag-based boundary rules."
        ),
        config_fragment,
        confidence: FindingConfidence::Medium,
        evidence: assignable_tags
            .iter()
            .map(|(name, glob, _)| format!("Tag `{name}` covers `{glob}`"))
            .collect(),
        rationale: Some(format!(
            "Adding these {tag_count} tag assignments to your `overrides` configuration \
             enables tag-based boundary rules. Tags abstract over directory structure, \
             making rules resilient to refactors. Once tags are assigned, you can write \
             rules like `{{\"from\": {{\"tag\": [\"role-components\"]}}, \"to\": {{\"tagNot\": [\"role-utils\"]}}}}` \
             to enforce architectural layering."
        )),
    });
}

// ---------------------------------------------------------------------------
// 3. Reachability fence suggestions
// ---------------------------------------------------------------------------

/// Suggest reachableFrom / reaches rules based on entrypoint structure.
fn suggest_reachability_fences(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    // Only suggest if rules are not already configured.
    if config.rules.is_some() {
        return;
    }

    // Detect packages that are only reachable from test entrypoints
    // and suggest a fence preventing production code from reaching them.
    let mut package_entrypoint_kinds: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File { package: Some(pkg), .. } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for ep_node in graph.module_graph.graph.node_indices() {
            if let ModuleNode::Entrypoint { kind, .. } = &graph.module_graph.graph[ep_node] {
                let kind_str = kind.as_str().to_string();
                package_entrypoint_kinds.entry(pkg.clone()).or_default().insert(kind_str);
            }
        }
    }

    let mut pkg_reachable_from_kinds: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();
    for entrypoint_info in &graph.entrypoints {
        let ep_kind = &entrypoint_info.kind;
        for file in &graph.files {
            if let Some(pkg) = &file.file.package
                && file.file.workspace.as_deref() == entrypoint_info.workspace.as_deref()
            {
                pkg_reachable_from_kinds.entry(pkg.clone()).or_default().insert(ep_kind.clone());
            }
        }
    }

    let test_like_kinds: FxHashSet<&str> =
        ["test", "vitest", "jest", "storybook", "story"].iter().copied().collect();

    for (pkg, kinds) in &pkg_reachable_from_kinds {
        if kinds.is_empty() {
            continue;
        }
        let all_test_like = kinds.iter().all(|k| test_like_kinds.contains(k.as_str()));
        if !all_test_like || kinds.is_empty() {
            continue;
        }

        let name = format!("fence-prod-from-{pkg}");
        let config_fragment = serde_json::json!({
            "rules": {
                "forbidden": [{
                    "name": name,
                    "severity": "error",
                    "comment": format!("Package `{pkg}` appears to be test-only; production code should not import it."),
                    "from": { "profiles": ["production"] },
                    "to": { "package": [pkg] }
                }]
            }
        });

        report.suggested_rules.push(SuggestedRule {
            kind: SuggestedRuleKind::ReachabilityFence,
            name,
            description: format!(
                "Package `{pkg}` is only reached via test-like entrypoints ({kinds_str}). \
                 A reachability fence prevents production code from accidentally importing it.",
                kinds_str = kinds.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            config_fragment,
            confidence: FindingConfidence::Medium,
            evidence: vec![format!(
                "Package `{pkg}` reached only by entrypoint kinds: {}",
                kinds.iter().cloned().collect::<Vec<_>>().join(", ")
            )],
            rationale: Some(
                "Test-only packages that leak into production bundles increase bundle size and \
                 can introduce flaky runtime behavior. A profile-scoped forbidden rule catches \
                 these leaks before they ship."
                    .to_string(),
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// 4. Layer enforcement suggestions
// ---------------------------------------------------------------------------

/// Detect implicit layering patterns and suggest layer enforcement rules.
fn suggest_layer_enforcement(graph: &GraphBuildResult, report: &mut SuggestRulesReport) {
    let mut workspace_deps: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File { workspace: Some(source_ws), .. } =
            &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            if let ModuleNode::File { workspace: Some(target_ws), .. } =
                &graph.module_graph.graph[edge.target()]
                && source_ws != target_ws
            {
                workspace_deps.entry(source_ws.clone()).or_default().insert(target_ws.clone());
            }
        }
    }

    if workspace_deps.len() < LAYER_CLUSTER_THRESHOLD {
        return;
    }

    let all_workspaces: FxHashSet<String> = workspace_deps
        .keys()
        .chain(workspace_deps.values().flat_map(|deps| deps.iter()))
        .cloned()
        .collect();

    let depended_upon: FxHashSet<String> =
        workspace_deps.values().flat_map(|deps| deps.iter()).cloned().collect();
    let has_outgoing: FxHashSet<String> = workspace_deps.keys().cloned().collect();

    let leaf_workspaces: Vec<String> = all_workspaces
        .iter()
        .filter(|ws| !has_outgoing.contains(*ws) && depended_upon.contains(*ws))
        .cloned()
        .collect();

    let top_workspaces: Vec<String> = all_workspaces
        .iter()
        .filter(|ws| has_outgoing.contains(*ws) && !depended_upon.contains(*ws))
        .cloned()
        .collect();

    if leaf_workspaces.is_empty() || top_workspaces.is_empty() {
        return;
    }

    for leaf in &leaf_workspaces {
        for top in &top_workspaces {
            let name = format!("layer-{leaf}-no-import-{top}");
            let config_fragment = serde_json::json!({
                "rules": {
                    "forbidden": [{
                        "name": name,
                        "severity": "error",
                        "comment": format!(
                            "Core workspace `{leaf}` should not depend on app workspace `{top}`."
                        ),
                        "from": { "workspace": [leaf] },
                        "to": { "workspace": [top] }
                    }]
                }
            });

            report.suggested_rules.push(SuggestedRule {
                kind: SuggestedRuleKind::LayerEnforcement,
                name,
                description: format!(
                    "Workspace `{leaf}` is a leaf dependency (core layer) while `{top}` is \
                     a top-level consumer (app layer). Enforcing this direction prevents \
                     circular workspace dependencies."
                ),
                config_fragment,
                confidence: FindingConfidence::Medium,
                evidence: vec![
                    format!("`{leaf}` has no cross-workspace imports (leaf/core layer)"),
                    format!("`{top}` is not imported by any other workspace (top/app layer)"),
                ],
                rationale: Some(format!(
                    "A clean workspace DAG prevents build-order cycles and keeps shared \
                     libraries independent of their consumers. If `{leaf}` imports from `{top}`, \
                     it creates a hidden coupling that breaks independent deployability."
                )),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Hotspot detection with ownership cross-referencing
// ---------------------------------------------------------------------------

/// Detect files with high fan-in / fan-out as hotspots, enriched with ownership data.
fn suggest_hotspots(
    graph: &GraphBuildResult,
    ownership_map: &FxHashMap<String, String>,
    report: &mut SuggestRulesReport,
) {
    for node_idx in graph.module_graph.graph.node_indices() {
        if let Some(hotspot) = compute_file_hotspot(graph, ownership_map, node_idx) {
            report.hotspots.push(hotspot);
        }
    }

    rank_and_truncate_hotspots(&mut report.hotspots);
}

fn compute_file_hotspot(
    graph: &GraphBuildResult,
    ownership_map: &FxHashMap<String, String>,
    node_idx: petgraph::graph::NodeIndex,
) -> Option<Hotspot> {
    let ModuleNode::File {
        relative_path,
        package: source_package,
        workspace: source_workspace,
        ..
    } = &graph.module_graph.graph[node_idx]
    else {
        return None;
    };

    let file_owner = ownership_map.get(relative_path);
    let mut outgoing = 0usize;
    let mut cross_package_out = 0usize;
    let mut cross_owner_out = 0usize;
    let mut teams_seen = FxHashSet::<String>::default();

    if let Some(owner) = file_owner {
        teams_seen.insert(owner.clone());
    }

    for edge in graph.module_graph.graph.edges(node_idx) {
        if !is_import_edge(*edge.weight()) {
            continue;
        }
        outgoing += 1;

        if let ModuleNode::File { package: Some(target_pkg), relative_path: target_path, .. } =
            &graph.module_graph.graph[edge.target()]
        {
            if source_package.as_ref() != Some(target_pkg) {
                cross_package_out += 1;
            }
            if let Some(target_owner) = ownership_map.get(target_path) {
                teams_seen.insert(target_owner.clone());
                if file_owner.is_some_and(|o| o != target_owner) {
                    cross_owner_out += 1;
                }
            }
        }
    }

    let mut incoming = 0usize;
    let mut cross_package_in = 0usize;
    let mut cross_owner_in = 0usize;

    for edge in graph.module_graph.graph.edges_directed(node_idx, petgraph::Direction::Incoming) {
        if !is_import_edge(*edge.weight()) {
            continue;
        }
        incoming += 1;

        if let ModuleNode::File {
            package: Some(importer_pkg), relative_path: importer_path, ..
        } = &graph.module_graph.graph[edge.source()]
        {
            if source_package.as_ref() != Some(importer_pkg) {
                cross_package_in += 1;
            }
            if let Some(importer_owner) = ownership_map.get(importer_path) {
                teams_seen.insert(importer_owner.clone());
                if file_owner.is_some_and(|o| o != importer_owner) {
                    cross_owner_in += 1;
                }
            }
        }
    }

    let total_edges = incoming + outgoing;
    let cross_package_imports = cross_package_in + cross_package_out;
    let cross_owner_imports = cross_owner_in + cross_owner_out;

    if total_edges < HOTSPOT_EDGE_THRESHOLD {
        return None;
    }

    let suggestion = describe_hotspot(
        relative_path,
        incoming,
        outgoing,
        cross_package_imports,
        cross_owner_imports,
        &teams_seen,
    );

    let mut teams_involved: Vec<String> = teams_seen.into_iter().collect();
    teams_involved.sort();

    Some(Hotspot {
        file: relative_path.clone(),
        workspace: source_workspace.clone(),
        package: source_package.clone(),
        cross_package_imports,
        cross_owner_imports,
        incoming_edges: incoming,
        outgoing_edges: outgoing,
        rank: 0,
        teams_involved,
        suggestion,
    })
}

fn describe_hotspot(
    relative_path: &str,
    incoming: usize,
    outgoing: usize,
    cross_package_imports: usize,
    cross_owner_imports: usize,
    teams_seen: &FxHashSet<String>,
) -> String {
    let total_edges = incoming + outgoing;

    if cross_owner_imports > 0 && cross_owner_imports > total_edges / 3 {
        format!(
            "`{relative_path}` has high cross-owner traffic ({cross_owner_imports} \
             cross-owner of {total_edges} total edges, {teams} teams involved). \
             Consider assigning shared ownership or extracting the shared surface \
             into a dedicated package with explicit API boundaries.",
            teams = teams_seen.len()
        )
    } else if cross_package_imports > total_edges / 2 {
        format!(
            "`{relative_path}` has high cross-package traffic ({cross_package_imports} \
             cross-package of {total_edges} total edges). Consider extracting a shared \
             package or adding boundary rules to control access."
        )
    } else if incoming > outgoing * 3 && outgoing > 0 {
        format!(
            "`{relative_path}` has very high fan-in ({incoming} incoming vs {outgoing} \
             outgoing). This file is a critical hub -- consider whether it should be an \
             explicit public API surface with a defined contract."
        )
    } else if outgoing > incoming * 3 && incoming > 0 {
        format!(
            "`{relative_path}` has very high fan-out ({outgoing} outgoing vs {incoming} \
             incoming). This file may be doing too much -- consider splitting into \
             focused modules."
        )
    } else {
        format!(
            "`{relative_path}` has high edge count ({total_edges} total, \
             fan-in: {incoming}, fan-out: {outgoing}). \
             Consider whether ownership assignment or boundary rules would help."
        )
    }
}

fn rank_and_truncate_hotspots(hotspots: &mut Vec<Hotspot>) {
    hotspots.sort_by(|a, b| {
        let total_a = a.incoming_edges + a.outgoing_edges;
        let total_b = b.incoming_edges + b.outgoing_edges;
        total_b.cmp(&total_a).then_with(|| b.cross_owner_imports.cmp(&a.cross_owner_imports))
    });

    hotspots.truncate(MAX_HOTSPOTS);
    for (i, hotspot) in hotspots.iter_mut().enumerate() {
        hotspot.rank = i + 1;
    }
}

// ---------------------------------------------------------------------------
// 6. Ownership hints with cross-team edges and package details
// ---------------------------------------------------------------------------

/// Suggest ownership hints by detecting directory clusters with high cross-package edges.
fn suggest_ownership_hints(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    ownership_map: &FxHashMap<String, String>,
    report: &mut SuggestRulesReport,
) {
    // If ownership is already fully configured, skip.
    if config.ownership.is_some() {
        report
            .rationale
            .push("Ownership configuration already present; skipping ownership hints.".to_string());
        return;
    }

    let mut dir_cross_edges: BTreeMap<String, usize> = BTreeMap::new();
    let mut dir_packages: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();
    let mut dir_cross_owner_edges: BTreeMap<String, usize> = BTreeMap::new();
    let mut dir_file_counts: BTreeMap<String, usize> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File { relative_path, package: Some(source_package), .. } =
            &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        let dir = directory_prefix(relative_path);
        *dir_file_counts.entry(dir.clone()).or_insert(0) += 1;
        let source_owner = ownership_map.get(relative_path);

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            if let ModuleNode::File {
                package: Some(target_package),
                relative_path: target_path,
                ..
            } = &graph.module_graph.graph[edge.target()]
                && source_package != target_package
            {
                *dir_cross_edges.entry(dir.clone()).or_insert(0) += 1;
                dir_packages.entry(dir.clone()).or_default().insert(target_package.clone());

                let target_owner = ownership_map.get(target_path);
                if source_owner.is_some() && target_owner.is_some() && source_owner != target_owner
                {
                    *dir_cross_owner_edges.entry(dir.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    for (dir, cross_edges) in &dir_cross_edges {
        if *cross_edges < BOUNDARY_EDGE_THRESHOLD {
            continue;
        }

        let touched_packages: Vec<String> = dir_packages.get(dir).map_or_else(Vec::new, |pkgs| {
            let mut v: Vec<String> = pkgs.iter().cloned().collect();
            v.sort();
            v
        });
        let target_packages = touched_packages.len();
        let cross_owner = dir_cross_owner_edges.get(dir).copied().unwrap_or(0);
        let file_count = dir_file_counts.get(dir).copied().unwrap_or(0);
        let suggested_owner = format!("team-{dir}");
        let path_glob = format!("{dir}/**");
        let rationale = format!(
            "Directory `{dir}` contains {file_count} file(s) and has {cross_edges} \
             cross-package import edges spanning {target_packages} other package(s)\
             {owner_note}. Assigning an owner enables ownership-boundary enforcement \
             and makes code review routing automatic.",
            owner_note = if cross_owner > 0 {
                format!(" ({cross_owner} of which cross existing ownership boundaries)")
            } else {
                String::new()
            }
        );

        report.ownership_hints.push(OwnershipHint {
            path_glob,
            suggested_owner,
            cross_team_edges: *cross_edges,
            touched_packages,
            rationale,
        });
    }
}

// ---------------------------------------------------------------------------
// 6b. Ownership suggestions from CODEOWNERS analysis
// ---------------------------------------------------------------------------

/// Analyze CODEOWNERS data to suggest ownership configuration.
fn suggest_ownership_from_codeowners(graph: &GraphBuildResult, report: &mut SuggestRulesReport) {
    let Some(codeowners) = &graph.discovery.codeowners else {
        return;
    };

    if codeowners.rules.is_empty() {
        return;
    }

    let mut owner_patterns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut owner_file_counts: BTreeMap<String, usize> = BTreeMap::new();

    for rule in &codeowners.rules {
        let owner_key = rule.owners.join(" ");
        if owner_key.is_empty() {
            continue;
        }
        owner_patterns.entry(owner_key.clone()).or_default().push(rule.pattern.clone());
    }

    for file in &graph.files {
        let relative_path = file.file.relative_path.to_string_lossy();
        if let Some(owners) = match_codeowners(codeowners, &relative_path) {
            *owner_file_counts.entry(owners).or_insert(0) += 1;
        }
    }

    let mut unowned_dirs: BTreeMap<String, usize> = BTreeMap::new();
    for file in &graph.files {
        let relative_path = file.file.relative_path.to_string_lossy();
        if match_codeowners(codeowners, &relative_path).is_none() {
            let dir = directory_prefix(&relative_path);
            *unowned_dirs.entry(dir).or_insert(0) += 1;
        }
    }

    if owner_patterns.len() >= 2 {
        let mut ownership_boundary_rule = serde_json::json!({
            "ownership": {
                "importCodeowners": true,
                "teams": {}
            }
        });

        if let Some(teams) = ownership_boundary_rule["ownership"]["teams"].as_object_mut() {
            for (owner, patterns) in &owner_patterns {
                let team_name = owner
                    .split_whitespace()
                    .next()
                    .unwrap_or(owner)
                    .trim_start_matches('@')
                    .replace('/', "-");

                teams.insert(
                    team_name.clone(),
                    serde_json::json!({
                        "paths": patterns,
                        "tags": [format!("team-{team_name}")]
                    }),
                );
            }
        }

        let owner_count = owner_patterns.len();
        let total_covered: usize = owner_file_counts.values().sum();

        report.suggested_rules.push(SuggestedRule {
            kind: SuggestedRuleKind::OwnershipBoundary,
            name: "import-codeowners-as-teams".to_string(),
            description: format!(
                "Import {owner_count} CODEOWNERS owner(s) as pruneguard teams covering \
                 {total_covered} file(s). This enables cross-team import tracking."
            ),
            config_fragment: ownership_boundary_rule,
            confidence: FindingConfidence::High,
            evidence: owner_file_counts
                .iter()
                .map(|(owner, count)| format!("Owner `{owner}` covers {count} file(s)"))
                .collect(),
            rationale: Some(format!(
                "Your repository has a CODEOWNERS file with {owner_count} distinct owner(s). \
                 Importing these as pruneguard teams with auto-assigned tags enables automatic \
                 cross-team boundary enforcement. When team A imports from team B's code, \
                 pruneguard can flag this for review -- catching accidental coupling before merge."
            )),
        });
    }

    for (dir, count) in &unowned_dirs {
        if *count < TAG_FILE_THRESHOLD {
            continue;
        }

        let already_hinted = report.ownership_hints.iter().any(|h| h.path_glob.starts_with(dir));
        if already_hinted {
            continue;
        }

        report.ownership_hints.push(OwnershipHint {
            path_glob: format!("{dir}/**"),
            suggested_owner: format!("team-{dir}"),
            cross_team_edges: 0,
            touched_packages: Vec::new(),
            rationale: format!(
                "Directory `{dir}` has {count} file(s) without CODEOWNERS coverage. \
                 Unowned code tends to accumulate tech debt because no team feels responsible \
                 for its quality. Add a CODEOWNERS rule or configure a pruneguard team."
            ),
        });
    }
}

// ---------------------------------------------------------------------------
// 7. Governance action synthesis
// ---------------------------------------------------------------------------

/// Synthesize prioritized governance actions from all gathered suggestions.
fn synthesize_governance_actions(report: &mut SuggestRulesReport) {
    let mut actions: Vec<GovernanceAction> = Vec::new();

    if let Some(action) = action_for_high_confidence_boundaries(&report.suggested_rules) {
        actions.push(action);
    }
    if let Some(action) = action_for_ownership_hints(&report.ownership_hints) {
        actions.push(action);
    }
    if let Some(action) = action_for_tags(&report.tags, &report.suggested_rules) {
        actions.push(action);
    }
    if let Some(action) = action_for_severe_hotspots(&report.hotspots) {
        actions.push(action);
    }
    if let Some(action) = action_for_layer_enforcement(&report.suggested_rules) {
        actions.push(action);
    }
    if let Some(action) = action_for_reachability_fences(&report.suggested_rules) {
        actions.push(action);
    }

    prioritize_actions(&mut actions);

    report.governance_actions = actions;
}

fn action_for_high_confidence_boundaries(
    suggested_rules: &[SuggestedRule],
) -> Option<GovernanceAction> {
    let high_confidence_rules: Vec<&SuggestedRule> =
        suggested_rules.iter().filter(|r| r.confidence == FindingConfidence::High).collect();
    if high_confidence_rules.is_empty() {
        return None;
    }

    let mut merged_forbidden: Vec<serde_json::Value> = Vec::new();
    for rule in &high_confidence_rules {
        if let Some(forbidden) = rule.config_fragment["rules"]["forbidden"].as_array() {
            merged_forbidden.extend(forbidden.iter().cloned());
        }
    }

    let config_fragment = if merged_forbidden.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "rules": {
                "forbidden": merged_forbidden
            }
        }))
    };

    Some(GovernanceAction {
        priority: 0,
        kind: GovernanceActionKind::AddBoundaryRule,
        description: format!(
            "Add {} high-confidence forbidden boundary rule(s). \
             These rules have the strongest evidence from actual import traffic.",
            high_confidence_rules.len()
        ),
        effort: EffortLevel::Low,
        impact: ImpactLevel::High,
        config_fragment,
    })
}

fn action_for_ownership_hints(ownership_hints: &[OwnershipHint]) -> Option<GovernanceAction> {
    if ownership_hints.is_empty() {
        return None;
    }

    let total_cross_edges: usize = ownership_hints.iter().map(|h| h.cross_team_edges).sum();

    let mut teams_obj = serde_json::Map::new();
    for hint in ownership_hints {
        let team_name = &hint.suggested_owner;
        teams_obj.insert(
            team_name.clone(),
            serde_json::json!({
                "paths": [&hint.path_glob],
                "tags": [team_name]
            }),
        );
    }

    let config_fragment = if teams_obj.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "ownership": {
                "teams": teams_obj
            }
        }))
    };

    Some(GovernanceAction {
        priority: 0,
        kind: GovernanceActionKind::AssignOwnership,
        description: format!(
            "Configure ownership for {} directory area(s) with {total_cross_edges} \
             total cross-boundary edges. Ownership enables cross-team import tracking \
             and automatic review routing.",
            ownership_hints.len()
        ),
        effort: EffortLevel::Medium,
        impact: ImpactLevel::High,
        config_fragment,
    })
}

fn action_for_tags(
    tags: &[SuggestedTag],
    suggested_rules: &[SuggestedRule],
) -> Option<GovernanceAction> {
    if tags.is_empty() {
        return None;
    }

    let workspace_tags = tags.iter().filter(|t| t.source.as_deref() == Some("workspace")).count();
    let package_tags = tags.iter().filter(|t| t.source.as_deref() == Some("package")).count();
    let dir_tags = tags.iter().filter(|t| t.source.as_deref() == Some("directory-cluster")).count();
    let owner_tags = tags.iter().filter(|t| t.source.as_deref() == Some("ownership")).count();
    let framework_tags =
        tags.iter().filter(|t| t.source.as_deref() == Some("framework-detection")).count();
    let pattern_tags = tags.iter().filter(|t| t.source.as_deref() == Some("file-pattern")).count();

    let mut details = Vec::new();
    if workspace_tags > 0 {
        details.push(format!("{workspace_tags} workspace"));
    }
    if package_tags > 0 {
        details.push(format!("{package_tags} package"));
    }
    if dir_tags > 0 {
        details.push(format!("{dir_tags} directory"));
    }
    if owner_tags > 0 {
        details.push(format!("{owner_tags} ownership"));
    }
    if framework_tags > 0 {
        details.push(format!("{framework_tags} framework"));
    }
    if pattern_tags > 0 {
        details.push(format!("{pattern_tags} file-pattern"));
    }

    let tag_assignment =
        suggested_rules.iter().find(|r| matches!(r.kind, SuggestedRuleKind::TagAssignment));
    let config_fragment = tag_assignment.map(|r| r.config_fragment.clone());

    Some(GovernanceAction {
        priority: 0,
        kind: GovernanceActionKind::IntroduceTags,
        description: format!(
            "Introduce {} tag(s) ({}) to enable tag-based boundary rules. \
             Tags decouple rules from directory structure.",
            tags.len(),
            details.join(", ")
        ),
        effort: EffortLevel::Low,
        impact: ImpactLevel::Medium,
        config_fragment,
    })
}

fn action_for_severe_hotspots(hotspots: &[Hotspot]) -> Option<GovernanceAction> {
    let severe_hotspots: Vec<&Hotspot> = hotspots
        .iter()
        .filter(|h| h.incoming_edges + h.outgoing_edges >= HOTSPOT_EDGE_THRESHOLD * 3)
        .collect();
    if severe_hotspots.is_empty() {
        return None;
    }

    Some(GovernanceAction {
        priority: 0,
        kind: GovernanceActionKind::SplitHotspot,
        description: format!(
            "{} file(s) have very high edge traffic (>{} edges). \
             Splitting these hotspots reduces coupling and makes ownership clearer. \
             Top hotspots: {}.",
            severe_hotspots.len(),
            HOTSPOT_EDGE_THRESHOLD * 3,
            severe_hotspots
                .iter()
                .take(3)
                .map(|h| format!("`{}` ({} edges)", h.file, h.incoming_edges + h.outgoing_edges))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        effort: EffortLevel::High,
        impact: ImpactLevel::High,
        config_fragment: None,
    })
}

fn action_for_layer_enforcement(suggested_rules: &[SuggestedRule]) -> Option<GovernanceAction> {
    let layer_rules: Vec<&SuggestedRule> = suggested_rules
        .iter()
        .filter(|r| matches!(r.kind, SuggestedRuleKind::LayerEnforcement))
        .collect();
    if layer_rules.is_empty() {
        return None;
    }

    let mut merged_forbidden: Vec<serde_json::Value> = Vec::new();
    for rule in &layer_rules {
        if let Some(forbidden) = rule.config_fragment["rules"]["forbidden"].as_array() {
            merged_forbidden.extend(forbidden.iter().cloned());
        }
    }

    let config_fragment = if merged_forbidden.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "rules": {
                "forbidden": merged_forbidden
            }
        }))
    };

    Some(GovernanceAction {
        priority: 0,
        kind: GovernanceActionKind::EnforceLayering,
        description: format!(
            "Add {} layer enforcement rule(s) to codify the workspace \
             dependency DAG. This prevents accidental circular dependencies between \
             workspaces.",
            layer_rules.len()
        ),
        effort: EffortLevel::Low,
        impact: ImpactLevel::Medium,
        config_fragment,
    })
}

fn action_for_reachability_fences(suggested_rules: &[SuggestedRule]) -> Option<GovernanceAction> {
    let fence_rules = suggested_rules
        .iter()
        .filter(|r| matches!(r.kind, SuggestedRuleKind::ReachabilityFence))
        .count();
    if fence_rules == 0 {
        return None;
    }

    Some(GovernanceAction {
        priority: 0,
        kind: GovernanceActionKind::AddReachabilityFence,
        description: format!(
            "Add {fence_rules} reachability fence(s) to prevent test-only code from \
             leaking into production bundles."
        ),
        effort: EffortLevel::Low,
        impact: ImpactLevel::Medium,
        config_fragment: None,
    })
}

fn prioritize_actions(actions: &mut [GovernanceAction]) {
    actions.sort_by(|a, b| {
        impact_ord(b.impact)
            .cmp(&impact_ord(a.impact))
            .then_with(|| effort_ord(a.effort).cmp(&effort_ord(b.effort)))
    });

    for (i, action) in actions.iter_mut().enumerate() {
        action.priority = i + 1;
    }
}

const fn impact_ord(level: ImpactLevel) -> u8 {
    match level {
        ImpactLevel::High => 3,
        ImpactLevel::Medium => 2,
        ImpactLevel::Low => 1,
    }
}

const fn effort_ord(level: EffortLevel) -> u8 {
    match level {
        EffortLevel::Low => 1,
        EffortLevel::Medium => 2,
        EffortLevel::High => 3,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if an edge represents an import (not structural edges like entrypoint-to-file).
const fn is_import_edge(edge: ModuleEdge) -> bool {
    matches!(
        edge,
        ModuleEdge::StaticImportValue
            | ModuleEdge::StaticImportType
            | ModuleEdge::DynamicImport
            | ModuleEdge::Require
            | ModuleEdge::SideEffectImport
            | ModuleEdge::ReExportNamed
            | ModuleEdge::ReExportAll
    )
}

/// Extract a directory pattern suitable for tag naming.
/// Takes the first two path segments if the path has at least 3 segments.
fn extract_tag_directory(relative_path: &str) -> Option<String> {
    let parts: Vec<&str> = relative_path.split('/').collect();
    if parts.len() >= 3 { Some(format!("{}/{}", parts[0], parts[1])) } else { None }
}

/// Extract the first directory segment of a relative path.
fn directory_prefix(relative_path: &str) -> String {
    relative_path.split('/').next().unwrap_or(relative_path).to_string()
}

fn compile_globset(patterns: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
            has_patterns = true;
        }
    }

    if !has_patterns {
        return None;
    }

    builder.build().ok()
}
