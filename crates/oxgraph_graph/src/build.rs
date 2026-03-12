use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{collections::hash_map::DefaultHasher, hash::Hasher};

use compact_str::CompactString;
use globset::{Glob, GlobSet, GlobSetBuilder};
use miette::{IntoDiagnostic, Result};
use oxgraph_cache::{
    AnalysisCache, CacheCounters, CachedFileFacts, CachedManifest, CachedResolutions,
    PathIndexEntry,
};
use oxgraph_config::{EntrypointsConfig, OxgraphConfig};
use oxgraph_discovery::{DiscoveryResult, discover};
use oxgraph_entrypoints::{EntrypointKind as SeedKind, EntrypointProfile, EntrypointSeed, detect_entrypoints};
use oxgraph_extract::{ExtractedFile, extract_file_facts};
use oxgraph_frameworks::built_in_packs;
use oxgraph_fs::{FileKind, FileRole, has_js_ts_extension};
use oxgraph_report::{
    EntrypointInfo, FileInfo, FileRole as ReportFileRole, Inventories, PackageInfo, Stats,
    WorkspaceInfo,
};
use oxgraph_resolver::{
    ModuleResolver, ResolutionOutcome, ResolvedEdge, ResolvedEdgeKind, dependency_name,
};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ModuleEdge, ModuleGraph, SymbolGraph};

/// Fully built repository graph and its supporting inventories.
#[derive(Debug)]
pub struct GraphBuildResult {
    pub discovery: DiscoveryResult,
    pub module_graph: ModuleGraph,
    pub symbol_graph: SymbolGraph,
    pub inventories: Inventories,
    pub entrypoints: Vec<EntrypointInfo>,
    pub entrypoint_seeds: Vec<EntrypointSeed>,
    pub files: Vec<ExtractedFile>,
    pub stats: Stats,
}

#[derive(Default, Clone, Copy)]
pub struct BuildOptions<'a> {
    pub cache: Option<&'a AnalysisCache>,
}

impl GraphBuildResult {
    /// Find a tracked file by absolute path, relative path, or suffix.
    pub fn find_file(&self, query: &str) -> Option<&ExtractedFile> {
        self.files
            .iter()
            .find(|file| {
                file.file.path == Path::new(query)
                    || file.file.relative_path == Path::new(query)
                    || file.file.relative_path.to_string_lossy() == query
                    || file.file.path.to_string_lossy() == query
                    || file.file.relative_path.to_string_lossy().ends_with(query)
            })
    }
}

/// Build the project graph for a scan/impact/explain run.
#[allow(clippy::too_many_lines)]
pub fn build_graph(
    cwd: &Path,
    config: &OxgraphConfig,
    scan_paths: &[PathBuf],
    profile: EntrypointProfile,
) -> Result<GraphBuildResult> {
    build_graph_with_options(cwd, config, scan_paths, profile, BuildOptions::default())
}

#[allow(clippy::too_many_lines)]
pub fn build_graph_with_options(
    cwd: &Path,
    config: &OxgraphConfig,
    scan_paths: &[PathBuf],
    profile: EntrypointProfile,
    options: BuildOptions<'_>,
) -> Result<GraphBuildResult> {
    let started = Instant::now();
    let scan_roots = normalize_scan_roots(cwd, scan_paths);
    let discovery_cwd = scan_roots.first().map_or_else(|| cwd.to_path_buf(), Clone::clone);
    let discovery = discover(&discovery_cwd, config)?;
    let exclude_matcher = compile_globset(&config.entrypoints.exclude);

    let mut files = discovery.collect_files(config);
    if !scan_roots.is_empty() {
        files.retain(|file| scan_roots.iter().any(|root| file.path.starts_with(root)));
    }

    let resolver = ModuleResolver::new(&config.resolver, &discovery.project_root);
    let config_hash = hash_json(config);
    let resolver_hash = hash_json(&config.resolver);
    let tsconfig_hash = compute_tsconfig_hash(&discovery.project_root, &files);
    let manifest_hashes = discovery
        .workspaces
        .iter()
        .map(|(name, workspace)| (name.clone(), hash_json(&workspace.manifest)))
        .collect::<FxHashMap<_, _>>();
    let mut cache_counters = CacheCounters::default();
    let mut extracted_files = files.into_iter().map(ExtractedFile::new).collect::<Vec<_>>();
    let repo_files = extracted_files
        .iter()
        .map(|file| file.file.path.clone())
        .collect::<FxHashSet<_>>();

    if let Some(cache) = options.cache {
        for (workspace_name, workspace) in &discovery.workspaces {
            let _ = cache.put_manifest(&CachedManifest {
                workspace: workspace_name.clone(),
                manifest_hash: *manifest_hashes.get(workspace_name).unwrap_or(&0),
                package_name: workspace.manifest.name.clone(),
                scripts_json: serde_json::to_vec(&workspace.manifest.scripts)
                    .unwrap_or_default(),
            });
        }
    }

    for extracted_file in &mut extracted_files {
        if let Some(cache) = options.cache {
            let manifest_hash = extracted_file
                .file
                .workspace
                .as_ref()
                .and_then(|workspace| manifest_hashes.get(workspace))
                .copied()
                .unwrap_or(0);
            let _ = cache.record_path_index(&PathIndexEntry {
                relative_path: extracted_file.file.relative_path.to_string_lossy().to_string(),
                absolute_path: extracted_file.file.path.to_string_lossy().to_string(),
                workspace: extracted_file.file.workspace.clone(),
                package: extracted_file.file.package.clone(),
                manifest_hash,
            });
        }

        populate_extracted_file(
            extracted_file,
            &resolver,
            &repo_files,
            options.cache,
            &mut cache_counters,
            config_hash,
            resolver_hash,
            tsconfig_hash,
            extracted_file
                .file
                .workspace
                .as_ref()
                .and_then(|workspace| manifest_hashes.get(workspace))
                .copied()
                .unwrap_or(0),
        )?;
    }

    let packs = built_in_packs();
    let mut entrypoint_seeds = detect_all_entrypoints(
        &discovery,
        &config.entrypoints,
        config.frameworks.as_ref(),
        &packs,
        exclude_matcher.as_ref(),
        &scan_roots,
    );
    let file_roles = extracted_files
        .iter()
        .map(|file| (file.file.path.clone(), file.file.role))
        .collect::<FxHashMap<_, _>>();
    entrypoint_seeds.retain(|seed| {
        let Some(role) = file_roles.get(&seed.path).copied() else {
            return false;
        };
        should_keep_entrypoint_seed(seed, role, config)
    });
    filter_entrypoints_by_profile(&mut entrypoint_seeds, profile);

    let inventories = build_inventories(&discovery, &extracted_files);
    let mut module_graph = ModuleGraph::new();
    let mut symbol_graph = SymbolGraph::default();

    let mut package_nodes = FxHashMap::default();
    for workspace in discovery.workspaces.values() {
        module_graph.add_workspace(&workspace.name, &workspace.root.to_string_lossy());
        let package_name = workspace
            .manifest
            .name
            .as_deref()
            .unwrap_or(workspace.name.as_str());
        let (_, package_index) = module_graph.add_package(
            package_name,
            Some(workspace.name.as_str()),
            &workspace.root.to_string_lossy(),
            workspace.manifest.version.as_deref(),
        );
        package_nodes.insert(workspace.name.clone(), package_index);
    }

    let mut file_nodes = FxHashMap::default();
    for extracted_file in &extracted_files {
        let (file_id, node) = module_graph.add_file(
            &extracted_file.file.path.to_string_lossy(),
            &extracted_file.file.relative_path.to_string_lossy(),
            extracted_file.file.workspace.as_deref(),
            extracted_file.file.package.as_deref(),
            extracted_file.file.kind,
            extracted_file.file.role,
        );
        file_nodes.insert(extracted_file.file.path.clone(), (file_id, node));

        if let Some(facts) = &extracted_file.facts {
            for export in &facts.exports {
                symbol_graph.add_export(file_id, export.name.clone(), export.is_type);
            }
        }
    }

    for extracted_file in &extracted_files {
        let Some((importer_id, importer_node)) = file_nodes.get(&extracted_file.file.path).copied() else {
            continue;
        };

        if let Some(facts) = &extracted_file.facts {
            for edge in &extracted_file.resolved_imports {
                add_resolved_edge(&mut module_graph, &file_nodes, importer_node, edge);
            }
            for edge in &extracted_file.resolved_reexports {
                add_resolved_edge(&mut module_graph, &file_nodes, importer_node, edge);
            }

            add_symbol_edges(&mut symbol_graph, importer_id, facts, &extracted_file.resolved_imports, &extracted_file.resolved_reexports, &file_nodes);
        }
    }

    let entrypoints = build_entrypoints(&mut module_graph, &entrypoint_seeds, &file_nodes, &package_nodes);
    seed_public_exports(&mut symbol_graph, &entrypoint_seeds, &file_nodes);

    let files_resolved = extracted_files
        .iter()
        .flat_map(|file| file.resolved_imports.iter().chain(&file.resolved_reexports))
        .filter(|edge| !matches!(edge.outcome, ResolutionOutcome::Unresolved))
        .count();
    let unresolved_specifiers = extracted_files
        .iter()
        .flat_map(|file| file.resolved_imports.iter().chain(&file.resolved_reexports))
        .filter(|edge| matches!(edge.outcome, ResolutionOutcome::Unresolved))
        .count();

    let stats = Stats {
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        files_parsed: extracted_files.iter().filter(|file| file.facts.is_some()).count(),
        files_cached: cache_counters.hits,
        files_discovered: extracted_files.len(),
        files_resolved,
        unresolved_specifiers,
        entrypoints_detected: entrypoint_seeds.len(),
        graph_nodes: module_graph.node_count(),
        graph_edges: module_graph.edge_count(),
        changed_files: 0,
        affected_files: 0,
        affected_packages: 0,
        affected_entrypoints: 0,
        baseline_applied: false,
        baseline_profile_mismatch: false,
        suppressed_findings: 0,
        new_findings: 0,
        cache_hits: cache_counters.hits,
        cache_misses: cache_counters.misses,
        cache_entries_read: cache_counters.entries_read,
        cache_entries_written: cache_counters.entries_written,
        affected_scope_incomplete: false,
    };

    Ok(GraphBuildResult {
        discovery,
        module_graph,
        symbol_graph,
        inventories,
        entrypoints,
        entrypoint_seeds,
        files: extracted_files,
        stats,
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn populate_extracted_file(
    extracted_file: &mut ExtractedFile,
    resolver: &ModuleResolver,
    repo_files: &FxHashSet<PathBuf>,
    cache: Option<&AnalysisCache>,
    cache_counters: &mut CacheCounters,
    config_hash: u64,
    resolver_hash: u64,
    tsconfig_hash: u64,
    manifest_hash: u64,
) -> Result<()> {
    if !has_js_ts_extension(&extracted_file.file.path) {
        return Ok(());
    }

    let source_bytes = std::fs::read(&extracted_file.file.path).into_diagnostic()?;
    let file_hash = hash_bytes(&source_bytes);
    if let Some(cache) = cache
        && hydrate_from_cache(
            extracted_file,
            cache,
            cache_counters,
            file_hash,
            config_hash,
            resolver_hash,
            manifest_hash,
            tsconfig_hash,
        )?
    {
        return Ok(());
    }

    let source = String::from_utf8(source_bytes).into_diagnostic()?;
    match extract_file_facts(&extracted_file.file.path, &source) {
        Ok(facts) => {
            extracted_file.external_dependencies.clear();

            for import in &facts.imports {
                let edge_kind = if import.is_side_effect {
                    ResolvedEdgeKind::SideEffectImport
                } else if import.is_type {
                    ResolvedEdgeKind::StaticImportType
                } else {
                    ResolvedEdgeKind::StaticImportValue
                };
                extracted_file.resolved_imports.push(resolve_edge(
                    resolver,
                    &extracted_file.file.path,
                    &import.specifier,
                    edge_kind,
                    import.line,
                    repo_files,
                ));
            }

            for reexport in &facts.reexports {
                let edge_kind = if reexport.is_star {
                    ResolvedEdgeKind::ReExportAll
                } else {
                    ResolvedEdgeKind::ReExportNamed
                };
                extracted_file.resolved_reexports.push(resolve_edge(
                    resolver,
                    &extracted_file.file.path,
                    &reexport.specifier,
                    edge_kind,
                    reexport.line,
                    repo_files,
                ));
            }

            for dynamic in &facts.dynamic_imports {
                if let Some(specifier) = &dynamic.specifier {
                    extracted_file.resolved_imports.push(resolve_edge(
                        resolver,
                        &extracted_file.file.path,
                        specifier,
                        ResolvedEdgeKind::DynamicImport,
                        dynamic.line,
                        repo_files,
                    ));
                }
            }

            for require in &facts.requires {
                if let Some(specifier) = &require.specifier {
                    extracted_file.resolved_imports.push(resolve_edge(
                        resolver,
                        &extracted_file.file.path,
                        specifier,
                        ResolvedEdgeKind::Require,
                        require.line,
                        repo_files,
                    ));
                }
            }

            extracted_file.external_dependencies = extracted_file
                .resolved_imports
                .iter()
                .chain(&extracted_file.resolved_reexports)
                .filter_map(|edge| edge.to_dependency.clone())
                .collect::<FxHashSet<_>>()
                .into_iter()
                .collect();
            extracted_file.external_dependencies.sort();
            extracted_file.facts = Some(facts);
        }
        Err(err) => {
            extracted_file.parse_diagnostics.push(err.to_string());
        }
    }

    if let Some(cache) = cache {
        persist_to_cache(
            extracted_file,
            cache,
            cache_counters,
            file_hash,
            config_hash,
            resolver_hash,
            manifest_hash,
            tsconfig_hash,
        )?;
    }

    Ok(())
}

fn resolve_edge(
    resolver: &ModuleResolver,
    from: &Path,
    specifier: &str,
    kind: ResolvedEdgeKind,
    line: u32,
    repo_files: &FxHashSet<PathBuf>,
) -> ResolvedEdge {
    match resolver.resolve(specifier, from) {
        Ok(module) if repo_files.contains(&module.path) => ResolvedEdge {
            from: from.to_path_buf(),
            specifier: specifier.to_string(),
            to_file: Some(module.path),
            to_dependency: None,
            kind,
            outcome: ResolutionOutcome::ResolvedToFile,
            unresolved_reason: None,
            via_exports: module.via_exports,
            line: Some(line),
        },
        Ok(module) => ResolvedEdge {
            from: from.to_path_buf(),
            specifier: specifier.to_string(),
            to_file: None,
            to_dependency: dependency_name(specifier).or_else(|| module.path.file_name().map(|name| name.to_string_lossy().to_string())),
            kind,
            outcome: ResolutionOutcome::ResolvedToDependency,
            unresolved_reason: None,
            via_exports: module.via_exports,
            line: Some(line),
        },
        Err(err) => ResolvedEdge {
            from: from.to_path_buf(),
            specifier: specifier.to_string(),
            to_file: None,
            to_dependency: dependency_name(specifier),
            kind,
            outcome: if dependency_name(specifier).is_some() {
                ResolutionOutcome::ResolvedToDependency
            } else {
                ResolutionOutcome::Unresolved
            },
            unresolved_reason: err.reason(),
            via_exports: false,
            line: Some(line),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn hydrate_from_cache(
    extracted_file: &mut ExtractedFile,
    cache: &AnalysisCache,
    counters: &mut CacheCounters,
    file_hash: u64,
    config_hash: u64,
    resolver_hash: u64,
    manifest_hash: u64,
    tsconfig_hash: u64,
) -> Result<bool> {
    counters.entries_read += 1;
    let Some(cached_file) = cache
        .get_file_facts(&extracted_file.file.path)
        .map_err(|err| miette::miette!("{err}"))?
    else {
        counters.misses += 1;
        return Ok(false);
    };
    counters.entries_read += 1;
    let Some(cached_resolutions) = cache
        .get_resolutions(&extracted_file.file.path)
        .map_err(|err| miette::miette!("{err}"))?
    else {
        counters.misses += 1;
        return Ok(false);
    };

    if cached_file.file_hash != file_hash
        || cached_file.config_hash != config_hash
        || cached_file.resolver_hash != resolver_hash
        || cached_file.manifest_hash != manifest_hash
        || cached_file.tsconfig_hash != tsconfig_hash
    {
        counters.misses += 1;
        return Ok(false);
    }

    extracted_file.facts = serde_json::from_slice(&cached_file.facts_json)
        .map_err(|err| miette::miette!("{err}"))?;
    extracted_file.parse_diagnostics = cached_file.parse_diagnostics;
    extracted_file.external_dependencies = cached_file.external_dependencies;
    extracted_file.resolved_imports = serde_json::from_slice(&cached_resolutions.resolved_imports_json)
        .map_err(|err| miette::miette!("{err}"))?;
    extracted_file.resolved_reexports =
        serde_json::from_slice(&cached_resolutions.resolved_reexports_json)
            .map_err(|err| miette::miette!("{err}"))?;
    counters.hits += 1;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn persist_to_cache(
    extracted_file: &ExtractedFile,
    cache: &AnalysisCache,
    counters: &mut CacheCounters,
    file_hash: u64,
    config_hash: u64,
    resolver_hash: u64,
    manifest_hash: u64,
    tsconfig_hash: u64,
) -> Result<()> {
    cache
        .put_file_facts(&CachedFileFacts {
            path: extracted_file.file.path.to_string_lossy().to_string(),
            relative_path: extracted_file.file.relative_path.to_string_lossy().to_string(),
            file_hash,
            config_hash,
            resolver_hash,
            manifest_hash,
            tsconfig_hash,
            facts_json: serde_json::to_vec(&extracted_file.facts)
                .map_err(|err| miette::miette!("{err}"))?,
            parse_diagnostics: extracted_file.parse_diagnostics.clone(),
            external_dependencies: extracted_file.external_dependencies.clone(),
        })
        .map_err(|err| miette::miette!("{err}"))?;
    counters.entries_written += 1;
    cache
        .put_resolutions(&CachedResolutions {
            path: extracted_file.file.path.to_string_lossy().to_string(),
            resolved_imports_json: serde_json::to_vec(&extracted_file.resolved_imports)
                .map_err(|err| miette::miette!("{err}"))?,
            resolved_reexports_json: serde_json::to_vec(&extracted_file.resolved_reexports)
                .map_err(|err| miette::miette!("{err}"))?,
        })
        .map_err(|err| miette::miette!("{err}"))?;
    counters.entries_written += 1;
    counters.misses += 1;
    Ok(())
}

fn detect_all_entrypoints(
    discovery: &DiscoveryResult,
    config: &EntrypointsConfig,
    frameworks_config: Option<&oxgraph_config::FrameworksConfig>,
    packs: &[Box<dyn oxgraph_frameworks::FrameworkPack>],
    exclude_matcher: Option<&GlobSet>,
    scan_roots: &[PathBuf],
) -> Vec<EntrypointSeed> {
    let mut entrypoints = Vec::new();

    for workspace in discovery.workspaces.values() {
        let mut workspace_entrypoints = detect_entrypoints(
            Some(workspace.name.as_str()),
            &workspace.root,
            &workspace.manifest,
            config,
            frameworks_config,
            packs,
        );

        workspace_entrypoints.retain(|entrypoint| {
            if !scan_roots.is_empty() && !scan_roots.iter().any(|root| entrypoint.path.starts_with(root)) {
                return false;
            }

            let relative = entrypoint
                .path
                .strip_prefix(&discovery.project_root)
                .unwrap_or(&entrypoint.path);
            if exclude_matcher.is_some_and(|matcher| matcher.is_match(relative)) {
                return false;
            }

            if !config.include_tests
                && relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".test.") || name.contains(".spec."))
            {
                return false;
            }

            if !config.include_stories
                && relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".stories.") || name.contains(".story."))
            {
                return false;
            }

            true
        });

        entrypoints.extend(workspace_entrypoints);
    }

    entrypoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.source.cmp(&b.source)));
    entrypoints.dedup_by(|left, right| left.path == right.path && left.profile == right.profile);
    entrypoints
}

fn filter_entrypoints_by_profile(entrypoints: &mut Vec<EntrypointSeed>, profile: EntrypointProfile) {
    entrypoints.retain(|entrypoint| match profile {
        EntrypointProfile::Both => true,
        EntrypointProfile::Production => {
            entrypoint.profile == EntrypointProfile::Production
                || entrypoint.profile == EntrypointProfile::Both
        }
        EntrypointProfile::Development => {
            entrypoint.profile == EntrypointProfile::Development
                || entrypoint.profile == EntrypointProfile::Both
        }
    });
}

fn should_keep_entrypoint_seed(
    seed: &EntrypointSeed,
    role: FileRole,
    config: &OxgraphConfig,
) -> bool {
    if seed.kind == SeedKind::ExplicitConfig {
        return true;
    }

    if role.excluded_from_auto_entrypoints() {
        return false;
    }

    if role == FileRole::Test && !config.entrypoints.include_tests {
        return false;
    }

    if role == FileRole::Story && !config.entrypoints.include_stories {
        return false;
    }

    true
}

fn build_inventories(discovery: &DiscoveryResult, files: &[ExtractedFile]) -> Inventories {
    let mut workspaces = discovery
        .workspaces
        .values()
        .map(|workspace| WorkspaceInfo {
            name: workspace.name.clone(),
            path: relative_to_project(&discovery.project_root, &workspace.root),
            package_count: 1,
        })
        .collect::<Vec<_>>();
    workspaces.sort_by(|a, b| a.name.cmp(&b.name));

    let mut packages = discovery
        .workspaces
        .values()
        .map(|workspace| PackageInfo {
            name: workspace
                .manifest
                .name
                .clone()
                .unwrap_or_else(|| workspace.name.clone()),
            version: workspace.manifest.version.clone(),
            workspace: workspace.name.clone(),
            path: relative_to_project(&discovery.project_root, &workspace.root),
        })
        .collect::<Vec<_>>();
    packages.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));

    let mut file_inventory = files
        .iter()
        .map(|file| FileInfo {
            path: file.file.relative_path.to_string_lossy().to_string(),
            workspace: file.file.workspace.clone(),
            kind: match file.file.kind {
                FileKind::Source => oxgraph_report::FileKind::Source,
                FileKind::Test => oxgraph_report::FileKind::Test,
                FileKind::Story => oxgraph_report::FileKind::Story,
                FileKind::Config => oxgraph_report::FileKind::Config,
                FileKind::Generated => oxgraph_report::FileKind::Generated,
                FileKind::BuildOutput => oxgraph_report::FileKind::BuildOutput,
            },
            role: Some(match file.file.role {
                FileRole::Source => ReportFileRole::Source,
                FileRole::Test => ReportFileRole::Test,
                FileRole::Story => ReportFileRole::Story,
                FileRole::Fixture => ReportFileRole::Fixture,
                FileRole::Example => ReportFileRole::Example,
                FileRole::Template => ReportFileRole::Template,
                FileRole::Benchmark => ReportFileRole::Benchmark,
                FileRole::Config => ReportFileRole::Config,
                FileRole::Generated => ReportFileRole::Generated,
                FileRole::BuildOutput => ReportFileRole::BuildOutput,
            }),
        })
        .collect::<Vec<_>>();
    file_inventory.sort_by(|a, b| a.path.cmp(&b.path));

    Inventories { files: file_inventory, packages, workspaces }
}

fn build_entrypoints(
    module_graph: &mut ModuleGraph,
    entrypoint_seeds: &[EntrypointSeed],
    file_nodes: &FxHashMap<PathBuf, (crate::FileId, petgraph::graph::NodeIndex)>,
    package_nodes: &FxHashMap<String, petgraph::graph::NodeIndex>,
) -> Vec<EntrypointInfo> {
    let mut entrypoints = Vec::new();

    for seed in entrypoint_seeds {
        let Some((file_id, file_node)) = file_nodes.get(&seed.path).copied() else {
            continue;
        };

        let kind = match seed.kind {
            SeedKind::PackageMain => crate::EntrypointKind::PackageMain,
            SeedKind::PackageBin => crate::EntrypointKind::PackageBin,
            SeedKind::PackageExports => crate::EntrypointKind::PackageExports,
            SeedKind::ExplicitConfig => crate::EntrypointKind::Explicit,
            SeedKind::FrameworkPack => crate::EntrypointKind::FrameworkDetected,
            SeedKind::Convention => crate::EntrypointKind::Convention,
            SeedKind::PackageScript => crate::EntrypointKind::PackageScript,
        };

        let entrypoint_node = module_graph.add_entrypoint(
            file_id,
            &seed.path.to_string_lossy(),
            kind,
            seed.profile,
            seed.workspace.as_deref(),
            &seed.source,
        );
        module_graph.add_edge(entrypoint_node, file_node, ModuleEdge::EntrypointToFile);
        if let Some(workspace) = &seed.workspace
            && let Some(package_node) = package_nodes.get(workspace).copied()
        {
            module_graph.add_edge(package_node, entrypoint_node, ModuleEdge::PackageToEntrypoint);
        }

        entrypoints.push(EntrypointInfo {
            path: seed.path.to_string_lossy().to_string(),
            kind: seed.kind.as_str().to_string(),
            profile: seed.profile.as_str().to_string(),
            workspace: seed.workspace.clone(),
            source: seed.source.clone(),
        });
    }

    entrypoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.kind.cmp(&b.kind)).then(a.source.cmp(&b.source)));
    entrypoints
}

fn seed_public_exports(
    symbol_graph: &mut SymbolGraph,
    entrypoint_seeds: &[EntrypointSeed],
    file_nodes: &FxHashMap<PathBuf, (crate::FileId, petgraph::graph::NodeIndex)>,
) {
    for seed in entrypoint_seeds {
        if let Some((file_id, _)) = file_nodes.get(&seed.path) {
            symbol_graph.mark_all_file_exports_live(*file_id, None);
        }
    }
}

fn add_resolved_edge(
    module_graph: &mut ModuleGraph,
    file_nodes: &FxHashMap<PathBuf, (crate::FileId, petgraph::graph::NodeIndex)>,
    importer_node: petgraph::graph::NodeIndex,
    edge: &ResolvedEdge,
) {
    if let Some(path) = &edge.to_file
        && let Some((_, target_node)) = file_nodes.get(path)
    {
        module_graph.add_edge(importer_node, *target_node, to_module_edge(edge.kind));
        return;
    }

    if let Some(dependency) = &edge.to_dependency {
        let dependency_node = module_graph.external_dependency_node(dependency);
        module_graph.add_edge(importer_node, dependency_node, ModuleEdge::FileToDependency);
    }
}

fn add_symbol_edges(
    symbol_graph: &mut SymbolGraph,
    importer_id: crate::FileId,
    facts: &oxgraph_extract::FileFacts,
    resolved_imports: &[ResolvedEdge],
    resolved_reexports: &[ResolvedEdge],
    file_nodes: &FxHashMap<PathBuf, (crate::FileId, petgraph::graph::NodeIndex)>,
) {
    for (import, edge) in facts.imports.iter().zip(resolved_imports.iter()) {
        let Some(source_path) = &edge.to_file else {
            continue;
        };
        let Some((source_id, _)) = file_nodes.get(source_path) else {
            continue;
        };

        if import.names.is_empty() {
            continue;
        }

        for name in &import.names {
            symbol_graph.add_import(
                importer_id,
                *source_id,
                name.imported.clone(),
                import.is_type,
            );
        }
    }

    for (reexport, edge) in facts.reexports.iter().zip(resolved_reexports.iter()) {
        let Some(source_path) = &edge.to_file else {
            continue;
        };
        let Some((source_id, _)) = file_nodes.get(source_path) else {
            continue;
        };

        if reexport.is_star {
            symbol_graph.add_reexport(
                importer_id,
                *source_id,
                CompactString::new("*"),
                CompactString::new("*"),
                true,
                reexport.is_type,
            );
            continue;
        }

        for name in &reexport.names {
            symbol_graph.add_reexport(
                importer_id,
                *source_id,
                name.original.clone(),
                name.exported.clone(),
                false,
                reexport.is_type,
            );
        }
    }
}

const fn to_module_edge(kind: ResolvedEdgeKind) -> ModuleEdge {
    match kind {
        ResolvedEdgeKind::StaticImportValue => ModuleEdge::StaticImportValue,
        ResolvedEdgeKind::StaticImportType => ModuleEdge::StaticImportType,
        ResolvedEdgeKind::DynamicImport => ModuleEdge::DynamicImport,
        ResolvedEdgeKind::Require => ModuleEdge::Require,
        ResolvedEdgeKind::SideEffectImport => ModuleEdge::SideEffectImport,
        ResolvedEdgeKind::ReExportNamed => ModuleEdge::ReExportNamed,
        ResolvedEdgeKind::ReExportAll => ModuleEdge::ReExportAll,
    }
}

fn normalize_scan_roots(cwd: &Path, scan_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = scan_paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            }
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
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

fn relative_to_project(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn compute_tsconfig_hash(project_root: &Path, files: &[oxgraph_fs::FileRecord]) -> u64 {
    let mut tsconfig_paths = files
        .iter()
        .map(|file| &file.relative_path)
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("tsconfig")
                        && Path::new(name)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    tsconfig_paths.sort();

    let mut hasher = DefaultHasher::new();
    for relative_path in tsconfig_paths {
        hasher.write(relative_path.to_string_lossy().as_bytes());
        if let Ok(bytes) = std::fs::read(project_root.join(&relative_path)) {
            hasher.write(&bytes);
        }
    }
    hasher.finish()
}

fn hash_json<T: serde::Serialize>(value: &T) -> u64 {
    match serde_json::to_vec(value) {
        Ok(bytes) => hash_bytes(&bytes),
        Err(_) => 0,
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}
