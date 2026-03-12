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

/// Analyze the graph and configuration to suggest governance rules.
pub fn suggest_rules(graph: &GraphBuildResult, config: &PruneguardConfig) -> SuggestRulesReport {
    let mut report = SuggestRulesReport::default();

    // Compute ownership map once for reuse across multiple suggestion passes.
    let ownership_map = build_ownership_map(graph, config.ownership.as_ref());

    suggest_boundary_rules(graph, config, &mut report);
    suggest_tags_from_directories(graph, &mut report);
    suggest_tags_from_structure(graph, config, &mut report);
    suggest_reachability_fences(graph, config, &mut report);
    suggest_layer_enforcement(graph, &mut report);
    suggest_hotspots(graph, &ownership_map, &mut report);
    suggest_ownership_hints(graph, config, &ownership_map, &mut report);
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
            if path_match || pkg_match {
                Some(matcher.team.clone())
            } else {
                None
            }
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

    Glob::new(&glob)
        .ok()
        .is_some_and(|compiled| compiled.compile_matcher().is_match(relative_path))
}

// ---------------------------------------------------------------------------
// 1. Boundary rules from cross-package traffic clusters
// ---------------------------------------------------------------------------

/// Suggest forbidden-dependency boundary rules by analyzing cross-package import edges.
fn suggest_boundary_rules(
    graph: &GraphBuildResult,
    config: &PruneguardConfig,
    report: &mut SuggestRulesReport,
) {
    // Already has rules configured -- skip boundary suggestions.
    if config.rules.is_some() {
        report.rationale.push(
            "Boundary rules already configured; skipping boundary suggestions.".to_string(),
        );
        return;
    }

    // Count cross-package edges: (source_package, target_package) -> count.
    let mut cross_package_edges: BTreeMap<(String, String), usize> = BTreeMap::new();
    // Also track distinct source files per pair for stronger evidence.
    let mut cross_package_files: BTreeMap<(String, String), FxHashSet<String>> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            package: Some(source_package),
            relative_path,
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }

            let target = edge.target();
            if let ModuleNode::File {
                package: Some(target_package),
                ..
            } = &graph.module_graph.graph[target]
                && source_package != target_package
            {
                let key = (source_package.clone(), target_package.clone());
                *cross_package_edges.entry(key.clone()).or_insert(0) += 1;
                cross_package_files
                    .entry(key)
                    .or_default()
                    .insert(relative_path.clone());
            }
        }
    }

    // Find heavily-crossed boundaries.
    for ((source, target), count) in &cross_package_edges {
        if *count < BOUNDARY_EDGE_THRESHOLD {
            continue;
        }

        let distinct_files = cross_package_files
            .get(&(source.clone(), target.clone()))
            .map_or(0, FxHashSet::len);

        let name = format!("forbid-{source}-to-{target}");
        let confidence = if *count >= BOUNDARY_EDGE_THRESHOLD * 3 {
            FindingConfidence::High
        } else if *count >= BOUNDARY_EDGE_THRESHOLD * 2 {
            FindingConfidence::Medium
        } else {
            FindingConfidence::Low
        };

        let config_fragment = serde_json::json!({
            "rules": {
                "forbidden": [{
                    "name": name,
                    "severity": "error",
                    "from": { "package": [source] },
                    "to": { "package": [target] }
                }]
            }
        });

        let rationale = format!(
            "Package `{source}` imports from `{target}` across {count} edges \
             from {distinct_files} distinct source file(s). A high edge count between \
             packages is a strong signal of a coupling boundary that should be made explicit. \
             Adding a forbidden rule lets you enforce this boundary in CI and catch regressions."
        );

        report.suggested_rules.push(SuggestedRule {
            kind: SuggestedRuleKind::Forbidden,
            name,
            description: format!(
                "Package `{source}` imports from `{target}` across {count} edges. \
                 Consider a forbidden rule if this crosses an intended boundary."
            ),
            config_fragment,
            confidence,
            evidence: vec![
                format!("{count} cross-package edges from `{source}` to `{target}`"),
                format!("{distinct_files} distinct source files participate in the coupling"),
            ],
            rationale: Some(rationale),
        });
    }

    if !cross_package_edges.is_empty() && report.suggested_rules.is_empty() {
        report.rationale.push(
            "Cross-package imports exist but none exceed the threshold for a boundary suggestion."
                .to_string(),
        );
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
    if !has_team_tags
        && let Some(teams) = config.ownership.as_ref().and_then(|o| o.teams.as_ref())
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

        // Walk backwards to entrypoints to determine which kinds reach this package.
        for ep_node in graph
            .module_graph
            .graph
            .node_indices()
        {
            if let ModuleNode::Entrypoint { kind, .. } = &graph.module_graph.graph[ep_node] {
                // Check if there is a path from this entrypoint to this file node.
                // For efficiency, we use the pre-computed reachability from entrypoints
                // by checking incoming edges transitively. Instead, we check the simpler
                // heuristic: entrypoint kind tags from the build's entrypoint seeds.
                let kind_str = kind.as_str().to_string();
                package_entrypoint_kinds
                    .entry(pkg.clone())
                    .or_default()
                    .insert(kind_str);
            }
        }
    }

    // Instead of full reachability (expensive), use the entrypoint-to-package association
    // from the graph structure: look at which entrypoint kinds have edges into each package.
    let mut pkg_reachable_from_kinds: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();
    for entrypoint_info in &graph.entrypoints {
        let ep_kind = &entrypoint_info.kind;
        // Walk files in this workspace to associate packages.
        for file in &graph.files {
            if let Some(pkg) = &file.file.package {
                // If this file's workspace matches the entrypoint's workspace, it's likely
                // reachable from this entrypoint kind (simplified heuristic).
                if file.file.workspace.as_deref() == entrypoint_info.workspace.as_deref() {
                    pkg_reachable_from_kinds
                        .entry(pkg.clone())
                        .or_default()
                        .insert(ep_kind.clone());
                }
            }
        }
    }

    // Find test-only packages: packages only reached by test/story entrypoints.
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
    // Look for workspace-level DAG patterns: if workspaces form clear layers
    // (e.g. apps -> packages/ui -> packages/core), suggest enforcing that layering.
    let mut workspace_deps: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            workspace: Some(source_ws),
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        for edge in graph.module_graph.graph.edges(node_idx) {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            if let ModuleNode::File {
                workspace: Some(target_ws),
                ..
            } = &graph.module_graph.graph[edge.target()]
                && source_ws != target_ws
            {
                workspace_deps
                    .entry(source_ws.clone())
                    .or_default()
                    .insert(target_ws.clone());
            }
        }
    }

    if workspace_deps.len() < LAYER_CLUSTER_THRESHOLD {
        return;
    }

    // Compute topological layers. Workspaces with no outgoing cross-workspace deps
    // are "leaf" / "core" layer. Others are higher layers.
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

    // Suggest that leaf (core) workspaces should not import from top (app) workspaces.
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
#[allow(clippy::too_many_lines)]
fn suggest_hotspots(
    graph: &GraphBuildResult,
    ownership_map: &FxHashMap<String, String>,
    report: &mut SuggestRulesReport,
) {
    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            relative_path,
            package: source_package,
            workspace: source_workspace,
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
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

            if let ModuleNode::File {
                package: Some(target_pkg),
                relative_path: target_path,
                ..
            } = &graph.module_graph.graph[edge.target()]
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

        for edge in graph
            .module_graph
            .graph
            .edges_directed(node_idx, petgraph::Direction::Incoming)
        {
            if !is_import_edge(*edge.weight()) {
                continue;
            }
            incoming += 1;

            if let ModuleNode::File {
                package: Some(importer_pkg),
                relative_path: importer_path,
                ..
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
            continue;
        }

        let suggestion = if cross_owner_imports > 0 && cross_owner_imports > total_edges / 3 {
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
        } else {
            format!(
                "`{relative_path}` has high edge count ({total_edges} total). \
                 Consider whether ownership assignment or boundary rules would help."
            )
        };

        let mut teams_involved: Vec<String> = teams_seen.into_iter().collect();
        teams_involved.sort();

        report.hotspots.push(Hotspot {
            file: relative_path.clone(),
            workspace: source_workspace.clone(),
            package: source_package.clone(),
            cross_package_imports,
            cross_owner_imports,
            incoming_edges: incoming,
            outgoing_edges: outgoing,
            rank: 0, // filled in after sorting
            teams_involved,
            suggestion,
        });
    }

    // Sort hotspots by total edges descending, then by cross-owner imports descending.
    report.hotspots.sort_by(|a, b| {
        let total_a = a.incoming_edges + a.outgoing_edges;
        let total_b = b.incoming_edges + b.outgoing_edges;
        total_b
            .cmp(&total_a)
            .then_with(|| b.cross_owner_imports.cmp(&a.cross_owner_imports))
    });

    // Assign ranks and truncate.
    report.hotspots.truncate(MAX_HOTSPOTS);
    for (i, hotspot) in report.hotspots.iter_mut().enumerate() {
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
        report.rationale.push(
            "Ownership configuration already present; skipping ownership hints.".to_string(),
        );
        return;
    }

    // Group cross-package edges by directory prefix of the source file.
    let mut dir_cross_edges: BTreeMap<String, usize> = BTreeMap::new();
    let mut dir_packages: BTreeMap<String, FxHashSet<String>> = BTreeMap::new();
    let mut dir_cross_owner_edges: BTreeMap<String, usize> = BTreeMap::new();

    for node_idx in graph.module_graph.graph.node_indices() {
        let ModuleNode::File {
            relative_path,
            package: Some(source_package),
            ..
        } = &graph.module_graph.graph[node_idx]
        else {
            continue;
        };

        let dir = directory_prefix(relative_path);
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
                dir_packages
                    .entry(dir.clone())
                    .or_default()
                    .insert(target_package.clone());

                // Track cross-owner edges.
                let target_owner = ownership_map.get(target_path);
                if source_owner.is_some()
                    && target_owner.is_some()
                    && source_owner != target_owner
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

        let touched_packages: Vec<String> =
            dir_packages.get(dir).map_or_else(Vec::new, |pkgs| {
                let mut v: Vec<String> = pkgs.iter().cloned().collect();
                v.sort();
                v
            });
        let target_packages = touched_packages.len();
        let cross_owner = dir_cross_owner_edges.get(dir).copied().unwrap_or(0);
        let suggested_owner = format!("team-{dir}");
        let path_glob = format!("{dir}/**");
        let rationale = format!(
            "Directory `{dir}` has {cross_edges} cross-package import edges \
             spanning {target_packages} other package(s){owner_note}. Assigning an owner \
             enables ownership-boundary enforcement and makes code review routing automatic.",
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
// 7. Governance action synthesis
// ---------------------------------------------------------------------------

/// Synthesize prioritized governance actions from all gathered suggestions.
#[allow(clippy::too_many_lines)]
fn synthesize_governance_actions(report: &mut SuggestRulesReport) {
    let mut actions: Vec<GovernanceAction> = Vec::new();

    // High-confidence boundary rules are the highest-value action.
    let high_confidence_rules = report
        .suggested_rules
        .iter()
        .filter(|r| r.confidence == FindingConfidence::High)
        .count();
    if high_confidence_rules > 0 {
        actions.push(GovernanceAction {
            priority: 0, // assigned after sorting
            kind: GovernanceActionKind::AddBoundaryRule,
            description: format!(
                "Add {high_confidence_rules} high-confidence forbidden boundary rule(s). \
                 These rules have the strongest evidence from actual import traffic."
            ),
            effort: EffortLevel::Low,
            impact: ImpactLevel::High,
            config_fragment: None,
        });
    }

    // If there are ownership hints, suggest adding ownership.
    if !report.ownership_hints.is_empty() {
        let total_cross_edges: usize = report.ownership_hints.iter().map(|h| h.cross_team_edges).sum();
        actions.push(GovernanceAction {
            priority: 0,
            kind: GovernanceActionKind::AssignOwnership,
            description: format!(
                "Configure ownership for {} directory area(s) with {total_cross_edges} \
                 total cross-boundary edges. Ownership enables cross-team import tracking \
                 and automatic review routing.",
                report.ownership_hints.len()
            ),
            effort: EffortLevel::Medium,
            impact: ImpactLevel::High,
            config_fragment: None,
        });
    }

    // If there are tag suggestions, suggest introducing tags.
    if !report.tags.is_empty() {
        let workspace_tags = report.tags.iter().filter(|t| t.source.as_deref() == Some("workspace")).count();
        let package_tags = report.tags.iter().filter(|t| t.source.as_deref() == Some("package")).count();
        let dir_tags = report.tags.iter().filter(|t| t.source.as_deref() == Some("directory-cluster")).count();
        let owner_tags = report.tags.iter().filter(|t| t.source.as_deref() == Some("ownership")).count();

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

        actions.push(GovernanceAction {
            priority: 0,
            kind: GovernanceActionKind::IntroduceTags,
            description: format!(
                "Introduce {} tag(s) ({}) to enable tag-based boundary rules. \
                 Tags decouple rules from directory structure.",
                report.tags.len(),
                details.join(", ")
            ),
            effort: EffortLevel::Low,
            impact: ImpactLevel::Medium,
            config_fragment: None,
        });
    }

    // If there are severe hotspots, suggest splitting them.
    let severe_hotspots = report
        .hotspots
        .iter()
        .filter(|h| h.incoming_edges + h.outgoing_edges >= HOTSPOT_EDGE_THRESHOLD * 3)
        .count();
    if severe_hotspots > 0 {
        actions.push(GovernanceAction {
            priority: 0,
            kind: GovernanceActionKind::SplitHotspot,
            description: format!(
                "{severe_hotspots} file(s) have very high edge traffic (>{} edges). \
                 Splitting these hotspots reduces coupling and makes ownership clearer.",
                HOTSPOT_EDGE_THRESHOLD * 3
            ),
            effort: EffortLevel::High,
            impact: ImpactLevel::High,
            config_fragment: None,
        });
    }

    // Layer enforcement suggestions.
    let layer_rules = report
        .suggested_rules
        .iter()
        .filter(|r| matches!(r.kind, SuggestedRuleKind::LayerEnforcement))
        .count();
    if layer_rules > 0 {
        actions.push(GovernanceAction {
            priority: 0,
            kind: GovernanceActionKind::EnforceLayering,
            description: format!(
                "Add {layer_rules} layer enforcement rule(s) to codify the workspace \
                 dependency DAG. This prevents accidental circular dependencies between \
                 workspaces."
            ),
            effort: EffortLevel::Low,
            impact: ImpactLevel::Medium,
            config_fragment: None,
        });
    }

    // Reachability fence suggestions.
    let fence_rules = report
        .suggested_rules
        .iter()
        .filter(|r| matches!(r.kind, SuggestedRuleKind::ReachabilityFence))
        .count();
    if fence_rules > 0 {
        actions.push(GovernanceAction {
            priority: 0,
            kind: GovernanceActionKind::AddReachabilityFence,
            description: format!(
                "Add {fence_rules} reachability fence(s) to prevent test-only code from \
                 leaking into production bundles."
            ),
            effort: EffortLevel::Low,
            impact: ImpactLevel::Medium,
            config_fragment: None,
        });
    }

    // Sort by impact (high first), then effort (low first).
    actions.sort_by(|a, b| {
        impact_ord(b.impact)
            .cmp(&impact_ord(a.impact))
            .then_with(|| effort_ord(a.effort).cmp(&effort_ord(b.effort)))
    });

    // Assign priority ranks.
    for (i, action) in actions.iter_mut().enumerate() {
        action.priority = i + 1;
    }

    report.governance_actions = actions;
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
    if parts.len() >= 3 {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
}

/// Extract the first directory segment of a relative path.
fn directory_prefix(relative_path: &str) -> String {
    relative_path
        .split('/')
        .next()
        .unwrap_or(relative_path)
        .to_string()
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
