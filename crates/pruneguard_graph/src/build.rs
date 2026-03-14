use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{collections::hash_map::DefaultHasher, hash::Hasher};

use rayon::prelude::*;

use compact_str::CompactString;
use globset::{Glob, GlobSet, GlobSetBuilder};
use miette::{IntoDiagnostic, Result};
use pruneguard_cache::{
    AnalysisCache, CacheCounters, CachedFileFacts, CachedManifest, CachedResolutions,
    PathIndexEntry,
};
use pruneguard_config::{EntrypointsConfig, PruneguardConfig};
use pruneguard_config_readers::{
    detect_route_entrypoints, extract_all_inputs, read_workspace_configs,
};
use pruneguard_discovery::{DiscoveryResult, discover};
use pruneguard_entrypoints::{
    EntrypointKind as SeedKind, EntrypointProfile, EntrypointSeed, EntrypointSurfaceKind,
    detect_entrypoints,
};
use pruneguard_extract::{AdapterOutput, ExtractedFile, MemberKind, extract_file_facts};
use pruneguard_frameworks::built_in_packs;
use pruneguard_fs::{FileKind, FileRole, is_tracked_source};
use pruneguard_report::{
    EntrypointInfo, FileInfo, FileRole as ReportFileRole, Inventories, PackageInfo, Stats,
    UnresolvedByReasonStats, WorkspaceInfo,
};
use pruneguard_resolver::{
    ModuleResolver, RESOLVER_LOGIC_VERSION, ResolutionOutcome, ResolvedEdge, ResolvedEdgeKind,
    dependency_name,
};
use pruneguard_semantic_client::{HelperDiscovery, SemanticClient, SemanticClientConfig};
use pruneguard_semantic_protocol::{QueryBatch, QueryKind, SemanticQuery};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{MemberAccessKind, MemberNodeKind, ModuleEdge, ModuleGraph, SymbolGraph};

/// Index of repo file paths that handles symlink differences transparently.
///
/// On macOS, `/tmp` is a symlink to `/private/tmp`, so the resolver may
/// return `/private/tmp/foo.ts` while the discovery walker reports
/// `/tmp/foo.ts`.  This struct stores both forms so `contains` and
/// `resolve_to_original` work regardless of which form is queried.
struct RepoFileIndex {
    paths: FxHashSet<PathBuf>,
    canonical_to_original: FxHashMap<PathBuf, PathBuf>,
}

impl RepoFileIndex {
    fn build(extracted_files: &[ExtractedFile]) -> Self {
        let mut paths = FxHashSet::default();
        let mut canonical_to_original = FxHashMap::default();
        for file in extracted_files {
            paths.insert(file.file.path.clone());
            if let Ok(canonical) = file.file.path.canonicalize()
                && canonical != file.file.path
            {
                paths.insert(canonical.clone());
                canonical_to_original.insert(canonical, file.file.path.clone());
            }
        }
        Self { paths, canonical_to_original }
    }

    /// Resolve a path to the original (non-canonical) form used by `file_nodes`.
    /// Returns the original form if a canonical→original mapping exists,
    /// otherwise returns the path as-is.
    fn to_original(&self, path: &PathBuf) -> PathBuf {
        self.canonical_to_original.get(path).cloned().unwrap_or_else(|| path.clone())
    }

    /// Look up a resolved module path, trying the raw form first and then
    /// the canonicalized form, and return the original repo path if found.
    fn match_resolved(&self, path: &Path) -> Option<PathBuf> {
        let pb = path.to_path_buf();
        if self.paths.contains(&pb) {
            return Some(self.to_original(&pb));
        }
        let canonical = path.canonicalize().ok()?;
        if self.paths.contains(&canonical) { Some(self.to_original(&canonical)) } else { None }
    }

    /// Iterate over the original (non-canonical) repo file paths.
    fn iter_original(&self) -> impl Iterator<Item = &PathBuf> {
        self.paths.iter().filter(|p| !self.canonical_to_original.contains_key(*p))
    }
}

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
    /// Files that are import targets of expanded glob/context patterns.
    /// Findings for exports in these files should use demoted confidence
    /// since their liveness depends on heuristic pattern matching.
    pub glob_expanded_targets: FxHashSet<PathBuf>,
    /// Alias prefixes extracted from framework configs (vite, webpack, tsconfig
    /// paths, etc.).  Used by the unused-dependency analyzer to filter unlisted
    /// dependency false positives from project-specific path aliases.
    pub config_alias_prefixes: Vec<String>,
    /// Fast path-to-file-index lookup for `find_file`.
    file_path_index: FxHashMap<String, usize>,
}

#[derive(Default, Clone, Copy)]
pub struct BuildOptions<'a> {
    pub cache: Option<&'a AnalysisCache>,
}

/// Grouped hash values used for cache invalidation.
#[derive(Clone, Copy)]
struct CacheHashes {
    config: u64,
    resolver: u64,
    tsconfig: u64,
    manifest: u64,
}

impl GraphBuildResult {
    /// Find a tracked file by absolute path, relative path, or suffix.
    pub fn find_file(&self, query: &str) -> Option<&ExtractedFile> {
        // Fast path: direct lookup by absolute or relative path.
        if let Some(&idx) = self.file_path_index.get(query) {
            return self.files.get(idx);
        }
        // Slow fallback for suffix matches (rare).
        self.files.iter().find(|file| file.file.relative_path.to_string_lossy().ends_with(query))
    }
}

/// Build the project graph for a scan/impact/explain run.
pub fn build_graph(
    cwd: &Path,
    config: &PruneguardConfig,
    scan_paths: &[PathBuf],
    profile: EntrypointProfile,
) -> Result<GraphBuildResult> {
    build_graph_with_options(cwd, config, scan_paths, profile, BuildOptions::default())
}

#[allow(clippy::too_many_lines)]
pub fn build_graph_with_options(
    cwd: &Path,
    config: &PruneguardConfig,
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
    let total_discovered_files = files.len();
    if !scan_roots.is_empty() {
        files.retain(|file| scan_roots.iter().any(|root| file.path.starts_with(root)));
    }
    let partial_scope = !scan_roots.is_empty() && files.len() < total_discovered_files;
    let partial_scope_reason = partial_scope.then(|| {
        format!(
            "scan paths narrowed analysis to {} of {} discovered files; dead-code findings are advisory for partial-scope scans",
            files.len(),
            total_discovered_files
        )
    });

    let mut resolver = ModuleResolver::new(&config.resolver, &discovery.project_root);

    // Build workspace package name → root directory mapping so the resolver
    // can handle deep subpath imports into workspace packages (e.g.
    // `@calcom/features/auth/lib/getLocale` → `packages/features/auth/lib/getLocale.ts`).
    let workspace_roots: FxHashMap<String, PathBuf> = discovery
        .workspaces
        .values()
        .filter_map(|ws| {
            let pkg_name = ws.manifest.name.as_ref()?;
            Some((pkg_name.clone(), ws.root.clone()))
        })
        .collect();
    resolver.set_workspace_roots(workspace_roots);

    // --- Phase 1: Read framework configs and feed into graph pipeline ---
    let mut all_workspace_configs = Vec::new();
    for workspace in discovery.workspaces.values() {
        all_workspace_configs.extend(read_workspace_configs(&workspace.root));
    }
    let mut config_inputs = extract_all_inputs(&all_workspace_configs);

    // --- Phase 1b: Detect route-generated entrypoints from package.json deps ---
    for workspace in discovery.workspaces.values() {
        let route_inputs = detect_route_entrypoints(&workspace.root);
        config_inputs.merge(route_inputs);
    }

    // Collect all aliases: config-derived + synthetic import map entries.
    let mut all_aliases: Vec<(String, String, pruneguard_resolver::AliasOrigin)> = config_inputs
        .aliases
        .iter()
        .map(|a| (a.pattern.clone(), a.target.clone(), a.origin))
        .collect();

    // Wire synthetic import maps as resolver aliases so auto-imported
    // symbols from Nuxt/Nitro generated .d.ts files create real edges.
    for sim in &config_inputs.synthetic_import_maps {
        let source_dir = sim.source_file.parent().unwrap_or_else(|| Path::new(""));
        for mapping in &sim.mappings {
            if let Some(ref resolved) = mapping.resolved_path {
                let target = if resolved.starts_with('.') {
                    source_dir.join(resolved).to_string_lossy().to_string()
                } else {
                    resolved.clone()
                };
                all_aliases.push((
                    mapping.import_name.clone(),
                    target,
                    pruneguard_resolver::AliasOrigin::FrameworkGenerated,
                ));
            }
        }
    }

    // Wire generated aliases (e.g. Nuxt auto-imported composables) as resolver
    // aliases so that usages of framework-generated symbols create real edges.
    for ga in &config_inputs.generated_aliases {
        all_aliases.push((
            ga.alias.clone(),
            ga.target.clone(),
            pruneguard_resolver::AliasOrigin::FrameworkGenerated,
        ));
    }

    if !all_aliases.is_empty() {
        resolver.set_config_aliases(all_aliases, &discovery.project_root);
    }

    // Feed config-derived externals to the resolver.
    if !config_inputs.externals.is_empty() {
        resolver.set_config_externals(config_inputs.externals.clone());
    }

    // Feed config-derived ignore_unresolved patterns to the resolver.
    // Also include virtual module prefixes.
    let mut ignore_patterns = config_inputs.ignore_unresolved.clone();
    ignore_patterns.extend(config_inputs.virtual_module_prefixes.clone());
    // Also register virtual module root prefixes so their specifiers
    // are not counted as unresolved.
    for vmr in &config_inputs.virtual_module_roots {
        ignore_patterns.push(vmr.prefix.clone());
    }
    ignore_patterns.sort();
    ignore_patterns.dedup();
    if !ignore_patterns.is_empty() {
        resolver.set_ignore_unresolved(ignore_patterns);
    }

    let base_hashes = CacheHashes {
        config: hash_json(config),
        resolver: {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hasher::write_u32(&mut h, RESOLVER_LOGIC_VERSION);
            std::hash::Hasher::write_u64(&mut h, hash_json(&config.resolver));
            std::hash::Hasher::finish(&h)
        },
        tsconfig: compute_tsconfig_hash(&discovery.project_root, &files),
        manifest: 0, // per-file override below
    };
    let manifest_hashes = discovery
        .workspaces
        .iter()
        .map(|(name, workspace)| (name.clone(), hash_json(&workspace.manifest)))
        .collect::<FxHashMap<_, _>>();
    let mut extracted_files = files.into_iter().map(ExtractedFile::new).collect::<Vec<_>>();
    let repo_files = RepoFileIndex::build(&extracted_files);

    // Phase 2a: Sequential cache hydration — read cached facts for files whose
    // hash hasn't changed. Track which files need fresh extraction.
    let mut cache_counters = CacheCounters::default();
    let mut per_file_hash: Vec<u64> = vec![0; extracted_files.len()];
    let mut needs_extract = vec![true; extracted_files.len()];
    if let Some(cache) = options.cache {
        for (i, extracted_file) in extracted_files.iter_mut().enumerate() {
            if !is_tracked_source(&extracted_file.file.path) {
                needs_extract[i] = false;
                continue;
            }
            let Ok(source_bytes) = std::fs::read(&extracted_file.file.path) else {
                continue;
            };
            let file_hash = hash_bytes(&source_bytes);
            per_file_hash[i] = file_hash;
            let hashes = CacheHashes {
                manifest: extracted_file
                    .file
                    .workspace
                    .as_ref()
                    .and_then(|workspace| manifest_hashes.get(workspace))
                    .copied()
                    .unwrap_or(0),
                ..base_hashes
            };
            if hydrate_from_cache(extracted_file, cache, &mut cache_counters, file_hash, hashes)
                .unwrap_or(false)
            {
                needs_extract[i] = false;
            }
        }
    }

    // Phase 2b: Parallel extraction for cache misses (no cache I/O)
    let errors: Vec<miette::Report> = extracted_files
        .par_iter_mut()
        .enumerate()
        .filter(|(i, _)| needs_extract[*i])
        .filter_map(|(_, extracted_file)| {
            extract_file(extracted_file, &resolver, &repo_files).err()
        })
        .collect();

    if let Some(first_error) = errors.into_iter().next() {
        return Err(first_error);
    }

    // Phase 2c: Batch cache persist — collect all entries and write in a single
    // transaction to avoid per-entry fsync overhead.
    if let Some(cache) = options.cache {
        let mut batch_facts = Vec::new();
        let mut batch_resolutions = Vec::new();
        for (i, extracted_file) in extracted_files.iter().enumerate() {
            if !needs_extract[i] || !is_tracked_source(&extracted_file.file.path) {
                continue;
            }
            let hashes = CacheHashes {
                manifest: extracted_file
                    .file
                    .workspace
                    .as_ref()
                    .and_then(|workspace| manifest_hashes.get(workspace))
                    .copied()
                    .unwrap_or(0),
                ..base_hashes
            };
            let file_hash = per_file_hash[i];
            batch_facts.push(CachedFileFacts {
                path: extracted_file.file.path.to_string_lossy().to_string(),
                relative_path: extracted_file.file.relative_path.to_string_lossy().to_string(),
                file_hash,
                config_hash: hashes.config,
                resolver_hash: hashes.resolver,
                manifest_hash: hashes.manifest,
                tsconfig_hash: hashes.tsconfig,
                facts_json: serde_json::to_vec(&extracted_file.facts).unwrap_or_default(),
                parse_diagnostics: extracted_file.parse_diagnostics.clone(),
                external_dependencies: extracted_file.external_dependencies.clone(),
            });
            batch_resolutions.push(CachedResolutions {
                path: extracted_file.file.path.to_string_lossy().to_string(),
                resolved_imports_json: serde_json::to_vec(&extracted_file.resolved_imports)
                    .unwrap_or_default(),
                resolved_reexports_json: serde_json::to_vec(&extracted_file.resolved_reexports)
                    .unwrap_or_default(),
            });
            cache_counters.entries_written += 2;
            cache_counters.misses += 1;
        }
        let path_entries: Vec<PathIndexEntry> = extracted_files
            .iter()
            .map(|f| PathIndexEntry {
                relative_path: f.file.relative_path.to_string_lossy().to_string(),
                absolute_path: f.file.path.to_string_lossy().to_string(),
                workspace: f.file.workspace.clone(),
                package: f.file.package.clone(),
                manifest_hash: f
                    .file
                    .workspace
                    .as_ref()
                    .and_then(|ws| manifest_hashes.get(ws))
                    .copied()
                    .unwrap_or(0),
            })
            .collect();
        let manifest_entries: Vec<CachedManifest> = discovery
            .workspaces
            .iter()
            .map(|(name, workspace)| CachedManifest {
                workspace: name.clone(),
                manifest_hash: *manifest_hashes.get(name).unwrap_or(&0),
                package_name: workspace.manifest.name.clone(),
                scripts_json: serde_json::to_vec(&workspace.manifest.scripts).unwrap_or_default(),
            })
            .collect();
        let _ = cache.put_extraction_batch(
            &batch_facts,
            &batch_resolutions,
            &path_entries,
            &manifest_entries,
        );
    }

    let packs = built_in_packs();
    let all_file_paths: Vec<PathBuf> =
        extracted_files.iter().map(|f| f.file.path.clone()).collect();
    let mut entrypoint_seeds = detect_all_entrypoints(
        &discovery,
        &config.entrypoints,
        config.frameworks.as_ref(),
        &packs,
        exclude_matcher.as_ref(),
        &scan_roots,
        &all_file_paths,
    );
    // --- Phase 1: Inject config-derived entrypoints ---
    inject_config_entrypoints(&mut entrypoint_seeds, &config_inputs, &discovery, &all_file_paths);

    // Route low-confidence reasons from framework config adapters into
    // report parity warnings so the output explains partial static coverage.
    for reason in &config_inputs.low_confidence_reasons {
        // These will be surfaced in stats.parity_warnings later.
        tracing::debug!(
            scope = %reason.scope,
            reason = %reason.reason,
            "low confidence: framework config adapter flagged partial coverage"
        );
    }

    // --- Phase 4: Seed from wildcard exports in manifests ---
    let wildcard_file_inventory: Vec<PathBuf> =
        extracted_files.iter().map(|f| f.file.relative_path.clone()).collect();
    let mut seen_seed_paths: FxHashSet<PathBuf> =
        entrypoint_seeds.iter().map(|s| s.path.clone()).collect();
    for workspace in discovery.workspaces.values() {
        let expanded = workspace.manifest.expand_wildcard_exports(&wildcard_file_inventory);
        for path in expanded {
            let abs = discovery.project_root.join(&path);
            if seen_seed_paths.insert(abs.clone()) {
                entrypoint_seeds.push(EntrypointSeed {
                    path: abs,
                    kind: SeedKind::PackageExports,
                    surface_kind: EntrypointSurfaceKind::PublicApi,
                    profile: EntrypointProfile::Both,
                    workspace: Some(workspace.name.clone()),
                    source: "wildcard-exports".to_string(),
                });
            }
        }
    }

    let file_roles = extracted_files
        .iter()
        .map(|file| (file.file.path.clone(), file.file.role))
        .collect::<FxHashMap<_, _>>();
    entrypoint_seeds.retain(|seed| {
        let Some(role) = file_roles.get(&seed.path).copied() else {
            // Framework-contributed entrypoints (e.g. story files discovered by
            // StorybookPack) may not appear in the file walker's inventory because
            // the walker and the framework pack traverse directories independently.
            // Keep them so they can still seed the graph.
            return seed.kind == SeedKind::FrameworkPack;
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
        let package_name = workspace.manifest.name.as_deref().unwrap_or(workspace.name.as_str());
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
        let Some((importer_id, importer_node)) = file_nodes.get(&extracted_file.file.path).copied()
        else {
            continue;
        };

        if let Some(facts) = &extracted_file.facts {
            for edge in &extracted_file.resolved_imports {
                add_resolved_edge(&mut module_graph, &file_nodes, importer_node, edge);
            }
            for edge in &extracted_file.resolved_reexports {
                add_resolved_edge(&mut module_graph, &file_nodes, importer_node, edge);
            }

            add_symbol_edges(
                &mut symbol_graph,
                importer_id,
                facts,
                &extracted_file.resolved_imports,
                &extracted_file.resolved_reexports,
                &file_nodes,
            );
        }
    }

    let entrypoints =
        build_entrypoints(&mut module_graph, &entrypoint_seeds, &file_nodes, &package_nodes);
    seed_public_exports_with_config(
        &mut symbol_graph,
        &entrypoint_seeds,
        &file_nodes,
        config.entrypoints.include_entry_exports,
    );

    // Propagate liveness through the symbol graph (import edges, re-export chains,
    // member references, and same-file references).
    symbol_graph.propagate_liveness();

    // --- Semantic refinement phase ---
    // Try to discover and spawn the semantic helper to refine dead-code findings.
    let mut semantic_used = false;
    let mut semantic_wall_ms: Option<u64> = None;
    let mut semantic_projects: Option<usize> = None;
    let mut semantic_files: Option<usize> = None;
    let mut semantic_queries_count: Option<usize> = None;
    let mut semantic_skipped_reason: Option<String> = None;
    let mut semantic_mode: Option<String> = None;

    let semantic_started = Instant::now();
    match SemanticClient::discover_binary(&discovery.project_root) {
        HelperDiscovery::Found(binary_path) => {
            // Collect tsconfig paths: prefer explicit config, fall back to auto-discovery.
            let tsconfig_paths: Vec<String> = if config.resolver.tsconfig.is_empty() {
                // Auto-discover tsconfig.json files from the project root.
                let mut paths = Vec::new();
                let root_tsconfig = discovery.project_root.join("tsconfig.json");
                if root_tsconfig.exists() {
                    paths.push(root_tsconfig.to_string_lossy().to_string());
                }
                for workspace in discovery.workspaces.values() {
                    let ws_tsconfig = workspace.root.join("tsconfig.json");
                    if ws_tsconfig.exists() && ws_tsconfig != root_tsconfig {
                        paths.push(ws_tsconfig.to_string_lossy().to_string());
                    }
                }
                paths
            } else {
                config
                    .resolver
                    .tsconfig
                    .iter()
                    .map(|p| {
                        let path = Path::new(p);
                        if path.is_absolute() {
                            p.clone()
                        } else {
                            discovery.project_root.join(p).to_string_lossy().to_string()
                        }
                    })
                    .collect()
            };

            match SemanticClient::spawn(
                &binary_path,
                &discovery.project_root.to_string_lossy(),
                tsconfig_paths.clone(),
                SemanticClientConfig::default(),
            ) {
                Ok(mut client) => {
                    semantic_mode = Some("auto".to_string());
                    let ready = client.ready_info();
                    semantic_projects = Some(ready.projects_loaded);
                    semantic_files = Some(ready.files_indexed);

                    if ready.projects_loaded > 0 {
                        semantic_used = true;

                        // Build reverse map: FileId -> absolute path string.
                        let file_id_to_path: FxHashMap<crate::ids::FileId, String> = file_nodes
                            .iter()
                            .map(|(path, (file_id, _))| {
                                (*file_id, path.to_string_lossy().to_string())
                            })
                            .collect();

                        // Query the semantic helper for exports that are not live.
                        // Walk the symbol graph to find exports with is_live == false,
                        // then ask the semantic helper to verify they're truly unreferenced.
                        let mut query_id = 0u64;
                        let mut queries = Vec::new();

                        for ((file_id, name), export) in &symbol_graph.exports {
                            if !export.is_live
                                && let Some(file_path) = file_id_to_path.get(file_id)
                            {
                                queries.push(SemanticQuery {
                                    id: query_id,
                                    kind: QueryKind::FindExportReferences,
                                    file_path: file_path.clone(),
                                    export_name: Some(name.to_string()),
                                    parent_name: None,
                                    member_name: None,
                                });
                                query_id += 1;
                            }
                        }

                        tracing::debug!(
                            queries = queries.len(),
                            "sending semantic refinement queries"
                        );

                        // Build a lookup so we can map query IDs back to their
                        // (FileId, export name) for marking exports live.
                        let query_index: Vec<(crate::ids::FileId, CompactString)> = queries
                            .iter()
                            .filter_map(|q| {
                                let name = q.export_name.as_ref()?;
                                let fid =
                                    file_nodes.get(Path::new(&q.file_path)).map(|(fid, _)| *fid)?;
                                Some((fid, CompactString::new(name)))
                            })
                            .collect();

                        // Send queries in batches.
                        let batch_size = client.ready_info().files_indexed.clamp(1, 128);
                        for chunk in queries.chunks(batch_size) {
                            let batch = QueryBatch {
                                queries: chunk.to_vec(),
                                tsconfig_path: tsconfig_paths.first().cloned().unwrap_or_default(),
                            };
                            match client.query(&batch) {
                                Ok(response) => {
                                    for result in &response.results {
                                        if result.success && result.total_references > 0 {
                                            // This export has references the syntactic
                                            // analysis missed — mark it as live.
                                            #[allow(clippy::cast_possible_truncation)]
                                            let idx = result.id as usize;
                                            if let Some((fid, export_name)) = query_index.get(idx) {
                                                symbol_graph.mark_live(*fid, export_name);
                                                tracing::debug!(
                                                    export = %export_name,
                                                    refs = result.total_references,
                                                    "semantic helper found references, marking live"
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("semantic query batch failed: {e}");
                                    break;
                                }
                            }
                        }

                        semantic_queries_count = Some(client.total_queries());
                        #[allow(clippy::cast_possible_truncation)]
                        {
                            semantic_wall_ms = Some(semantic_started.elapsed().as_millis() as u64);
                        }
                    } else {
                        semantic_skipped_reason = Some("no TypeScript projects loaded".to_string());
                    }

                    let _ = client.shutdown();
                }
                Err(e) => {
                    semantic_skipped_reason = Some(format!("spawn failed: {e}"));
                }
            }
        }
        HelperDiscovery::NotFound(reason) => {
            semantic_skipped_reason = Some(reason);
        }
    }

    let mut files_resolved = 0;
    let mut unresolved_specifiers = 0;
    let mut resolved_via_exports = 0;
    let mut unresolved_by_reason = UnresolvedByReasonStats::default();
    for edge in extracted_files
        .iter()
        .flat_map(|file| file.resolved_imports.iter().chain(&file.resolved_reexports))
    {
        match edge.outcome {
            ResolutionOutcome::ResolvedToFile | ResolutionOutcome::ResolvedToDependency => {
                files_resolved += 1;
                if edge.via_exports {
                    resolved_via_exports += 1;
                }
            }
            ResolutionOutcome::Unresolved => {
                unresolved_specifiers += 1;
                match edge.unresolved_reason {
                    Some(pruneguard_resolver::UnresolvedReason::UnsupportedSpecifier) => {
                        unresolved_by_reason.unsupported_specifier += 1;
                    }
                    Some(pruneguard_resolver::UnresolvedReason::TsconfigPathMiss) => {
                        unresolved_by_reason.tsconfig_path_miss += 1;
                    }
                    Some(pruneguard_resolver::UnresolvedReason::ExportsConditionMiss) => {
                        unresolved_by_reason.exports_condition_miss += 1;
                    }
                    Some(pruneguard_resolver::UnresolvedReason::Externalized) => {
                        unresolved_by_reason.externalized += 1;
                    }
                    Some(pruneguard_resolver::UnresolvedReason::WorkspaceExportsMiss) => {
                        unresolved_by_reason.workspace_exports_miss += 1;
                    }
                    Some(pruneguard_resolver::UnresolvedReason::MissingFile) | None => {
                        unresolved_by_reason.missing_file += 1;
                    }
                }
            }
        }
    }

    let stats = Stats {
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        files_parsed: extracted_files.iter().filter(|file| file.facts.is_some()).count(),
        files_cached: cache_counters.hits,
        files_discovered: extracted_files.len(),
        files_resolved,
        unresolved_specifiers,
        unresolved_by_reason,
        resolved_via_exports,
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
        focus_applied: false,
        focused_files: 0,
        focused_findings: 0,
        full_scope_required: false,
        partial_scope,
        partial_scope_reason,
        confidence_counts: pruneguard_report::ConfidenceCounts::default(),
        parity_warnings: config_inputs
            .low_confidence_reasons
            .iter()
            .map(|r| format!("[{}] {}", r.scope, r.reason))
            .collect(),
        cache_hits: cache_counters.hits,
        cache_misses: cache_counters.misses,
        cache_entries_read: cache_counters.entries_read,
        cache_entries_written: cache_counters.entries_written,
        affected_scope_incomplete: false,
        execution_mode: None,
        index_warm: None,
        index_age_ms: None,
        reused_graph_nodes: None,
        reused_graph_edges: None,
        watcher_lag_ms: None,
        frameworks_detected: Vec::new(),
        heuristic_frameworks: Vec::new(),
        heuristic_entrypoints: 0,
        compatibility_warnings: Vec::new(),
        strict_trust_applied: false,
        framework_confidence_counts: pruneguard_report::FrameworkConfidenceCounts::default(),
        unsupported_frameworks: Vec::new(),
        external_parity_pct: None,
        external_parity: None,
        semantic_mode,
        semantic_used,
        semantic_wall_ms,
        semantic_projects,
        semantic_files,
        semantic_queries: semantic_queries_count,
        semantic_skipped_reason,
        replacement_score: None,
        replacement_family_scores: Vec::new(),
    };

    // Build fast path-to-file index for O(1) lookups.
    let mut file_path_index = FxHashMap::default();
    for (i, f) in extracted_files.iter().enumerate() {
        file_path_index.insert(f.file.path.to_string_lossy().to_string(), i);
        file_path_index.insert(f.file.relative_path.to_string_lossy().to_string(), i);
    }

    // Collect files that were pulled in via glob/context expansion for confidence demotion.
    // Collect alias prefixes for the unused-dependency analyzer.
    // Strip trailing /* or * to get the prefix that can be matched against
    // specifiers (e.g., "@shared/*" → "@shared", "~/src/*" → "~/src").
    let config_alias_prefixes: Vec<String> = config_inputs
        .aliases
        .iter()
        .map(|a| {
            a.pattern
                .strip_suffix("/*")
                .or_else(|| a.pattern.strip_suffix('*'))
                .unwrap_or(&a.pattern)
                .to_string()
        })
        .collect();

    let glob_expanded_targets: FxHashSet<PathBuf> = extracted_files
        .iter()
        .flat_map(|f| f.resolved_imports.iter())
        .filter(|edge| {
            matches!(edge.kind, ResolvedEdgeKind::ImportMetaGlob | ResolvedEdgeKind::RequireContext)
        })
        .filter_map(|edge| edge.to_file.clone())
        .collect();

    Ok(GraphBuildResult {
        discovery,
        module_graph,
        symbol_graph,
        inventories,
        entrypoints,
        entrypoint_seeds,
        files: extracted_files,
        stats,
        glob_expanded_targets,
        config_alias_prefixes,
        file_path_index,
    })
}

#[allow(clippy::too_many_lines)]
fn extract_file(
    extracted_file: &mut ExtractedFile,
    resolver: &ModuleResolver,
    repo_files: &RepoFileIndex,
) -> Result<()> {
    if !is_tracked_source(&extracted_file.file.path) {
        return Ok(());
    }

    let source_bytes = std::fs::read(&extracted_file.file.path).into_diagnostic()?;
    let source = String::from_utf8(source_bytes).into_diagnostic()?;
    match extract_file_facts(&extracted_file.file.path, &source) {
        Ok(AdapterOutput {
            facts, synthetic_imports, synthetic_reexports, diagnostics, ..
        }) => {
            extracted_file.external_dependencies.clear();
            extracted_file.parse_diagnostics.extend(diagnostics.iter().map(|d| d.message.clone()));

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

            // Resolve dependency_patterns into real graph edges.
            for pattern in &facts.dependency_patterns {
                match pattern {
                    pruneguard_extract::DependencyPattern::RequireResolve { specifier, line } => {
                        extracted_file.resolved_imports.push(resolve_edge(
                            resolver,
                            &extracted_file.file.path,
                            specifier,
                            ResolvedEdgeKind::RequireResolve,
                            *line,
                            repo_files,
                        ));
                    }
                    pruneguard_extract::DependencyPattern::ImportMetaGlob {
                        pattern: glob_pattern,
                        line,
                    } => {
                        if glob_pattern.contains('*') || glob_pattern.contains('?') {
                            // Expand wildcard glob against tracked source inventory.
                            expand_glob_into_edges(
                                extracted_file,
                                glob_pattern,
                                &[],
                                *line,
                                resolver,
                                repo_files,
                            );
                        } else {
                            extracted_file.resolved_imports.push(resolve_edge(
                                resolver,
                                &extracted_file.file.path,
                                glob_pattern,
                                ResolvedEdgeKind::ImportMetaGlob,
                                *line,
                                repo_files,
                            ));
                        }
                    }
                    pruneguard_extract::DependencyPattern::TripleSlashReference {
                        path: ref_path,
                        is_types,
                        line,
                    } => {
                        let edge_kind = if *is_types {
                            ResolvedEdgeKind::TripleSlashTypes
                        } else {
                            ResolvedEdgeKind::TripleSlashFile
                        };
                        extracted_file.resolved_imports.push(resolve_edge(
                            resolver,
                            &extracted_file.file.path,
                            ref_path,
                            edge_kind,
                            *line,
                            repo_files,
                        ));
                    }
                    pruneguard_extract::DependencyPattern::JsDocImport { specifier, line } => {
                        extracted_file.resolved_imports.push(resolve_edge(
                            resolver,
                            &extracted_file.file.path,
                            specifier,
                            ResolvedEdgeKind::JsDocImport,
                            *line,
                            repo_files,
                        ));
                    }
                    pruneguard_extract::DependencyPattern::ImportMetaResolve {
                        specifier,
                        line,
                    } => {
                        extracted_file.resolved_imports.push(resolve_edge(
                            resolver,
                            &extracted_file.file.path,
                            specifier,
                            ResolvedEdgeKind::ImportMetaResolve,
                            *line,
                            repo_files,
                        ));
                    }
                    pruneguard_extract::DependencyPattern::RequireContext {
                        directory,
                        recursive,
                        regex_filter,
                        line,
                    } => {
                        // Expand require.context directory against tracked source inventory.
                        expand_require_context_into_edges(
                            extracted_file,
                            directory,
                            *recursive,
                            regex_filter.as_deref(),
                            *line,
                            resolver,
                            repo_files,
                        );
                    }
                    pruneguard_extract::DependencyPattern::UrlConstructor { specifier, line } => {
                        extracted_file.resolved_imports.push(resolve_edge(
                            resolver,
                            &extracted_file.file.path,
                            specifier,
                            ResolvedEdgeKind::UrlConstructor,
                            *line,
                            repo_files,
                        ));
                    }
                    pruneguard_extract::DependencyPattern::ImportEquals { specifier, line } => {
                        extracted_file.resolved_imports.push(resolve_edge(
                            resolver,
                            &extracted_file.file.path,
                            specifier,
                            ResolvedEdgeKind::ImportEquals,
                            *line,
                            repo_files,
                        ));
                    }
                    pruneguard_extract::DependencyPattern::ImportMetaGlobArray {
                        patterns,
                        line,
                    } => {
                        // Separate positive patterns from negation patterns (prefixed with `!`).
                        let negations: Vec<&str> = patterns
                            .iter()
                            .filter(|p| p.starts_with('!'))
                            .map(String::as_str)
                            .collect();
                        for glob_pattern in patterns {
                            if glob_pattern.starts_with('!') {
                                continue; // Negations are applied as filters, not expanded.
                            }
                            if glob_pattern.contains('*') || glob_pattern.contains('?') {
                                expand_glob_into_edges(
                                    extracted_file,
                                    glob_pattern,
                                    &negations,
                                    *line,
                                    resolver,
                                    repo_files,
                                );
                            } else {
                                extracted_file.resolved_imports.push(resolve_edge(
                                    resolver,
                                    &extracted_file.file.path,
                                    glob_pattern,
                                    ResolvedEdgeKind::ImportMetaGlob,
                                    *line,
                                    repo_files,
                                ));
                            }
                        }
                    }
                }
            }

            // Resolve synthetic imports generated by source adapters (e.g. template
            // component references in Vue/Svelte/Astro/MDX).
            for synthetic in &synthetic_imports {
                extracted_file.resolved_imports.push(resolve_edge(
                    resolver,
                    &extracted_file.file.path,
                    &synthetic.specifier,
                    ResolvedEdgeKind::StaticImportValue,
                    synthetic.line,
                    repo_files,
                ));
            }
            for synthetic in &synthetic_reexports {
                extracted_file.resolved_reexports.push(resolve_edge(
                    resolver,
                    &extracted_file.file.path,
                    &synthetic.specifier,
                    ResolvedEdgeKind::ReExportNamed,
                    synthetic.line,
                    repo_files,
                ));
            }

            extracted_file.external_dependencies = extracted_file
                .resolved_imports
                .iter()
                .chain(&extracted_file.resolved_reexports)
                .filter(|edge| {
                    !matches!(
                        edge.alias_origin,
                        Some(
                            pruneguard_resolver::AliasOrigin::Vite
                                | pruneguard_resolver::AliasOrigin::Webpack
                                | pruneguard_resolver::AliasOrigin::Babel
                                | pruneguard_resolver::AliasOrigin::TsconfigPaths
                                | pruneguard_resolver::AliasOrigin::FrameworkGenerated
                        )
                    )
                })
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

    Ok(())
}

fn resolve_edge(
    resolver: &ModuleResolver,
    from: &Path,
    specifier: &str,
    kind: ResolvedEdgeKind,
    line: u32,
    repo_files: &RepoFileIndex,
) -> ResolvedEdge {
    match resolver.resolve(specifier, from) {
        Ok(module) => {
            // Try to match the resolved path against repo_files.  The resolver
            // may return a path that differs from the repo_files entry only by
            // a symlink prefix (e.g. /tmp vs /private/tmp on macOS).
            // match_resolved handles canonicalization and maps back to the
            // original form used by file_nodes.
            if let Some(repo_path) = repo_files.match_resolved(&module.path) {
                ResolvedEdge {
                    from: from.to_path_buf(),
                    specifier: specifier.to_string(),
                    to_file: Some(repo_path),
                    // When a bare specifier (e.g. `@wordwar/core`) resolves to a file
                    // inside the repo (cross-workspace import), also record the
                    // dependency name so the unused-dependency analyzer knows the
                    // declared package.json dependency is in use.
                    to_dependency: dependency_name(specifier),
                    kind,
                    outcome: ResolutionOutcome::ResolvedToFile,
                    unresolved_reason: None,
                    via_exports: module.via_exports,
                    exports_subpath: module.exports_subpath,
                    exports_condition: module.exports_condition,
                    alias_origin: module.alias_origin,
                    line: Some(line),
                }
            } else {
                let canonical_path =
                    module.path.canonicalize().unwrap_or_else(|_| module.path.clone());
                ResolvedEdge {
                    from: from.to_path_buf(),
                    specifier: specifier.to_string(),
                    to_file: None,
                    to_dependency: dependency_name(specifier).or_else(|| {
                        canonical_path.file_name().map(|name| name.to_string_lossy().to_string())
                    }),
                    kind,
                    outcome: ResolutionOutcome::ResolvedToDependency,
                    unresolved_reason: None,
                    via_exports: module.via_exports,
                    exports_subpath: module.exports_subpath,
                    exports_condition: module.exports_condition,
                    alias_origin: module.alias_origin,
                    line: Some(line),
                }
            }
        }
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
            exports_subpath: None,
            exports_condition: None,
            alias_origin: None,
            line: Some(line),
        },
    }
}

fn hydrate_from_cache(
    extracted_file: &mut ExtractedFile,
    cache: &AnalysisCache,
    counters: &mut CacheCounters,
    file_hash: u64,
    hashes: CacheHashes,
) -> Result<bool> {
    counters.entries_read += 1;
    let Some(cached_file) =
        cache.get_file_facts(&extracted_file.file.path).map_err(|err| miette::miette!("{err}"))?
    else {
        counters.misses += 1;
        return Ok(false);
    };
    counters.entries_read += 1;
    let Some(cached_resolutions) =
        cache.get_resolutions(&extracted_file.file.path).map_err(|err| miette::miette!("{err}"))?
    else {
        counters.misses += 1;
        return Ok(false);
    };

    if cached_file.file_hash != file_hash
        || cached_file.config_hash != hashes.config
        || cached_file.resolver_hash != hashes.resolver
        || cached_file.manifest_hash != hashes.manifest
        || cached_file.tsconfig_hash != hashes.tsconfig
    {
        counters.misses += 1;
        return Ok(false);
    }

    extracted_file.facts =
        serde_json::from_slice(&cached_file.facts_json).map_err(|err| miette::miette!("{err}"))?;
    extracted_file.parse_diagnostics = cached_file.parse_diagnostics;
    extracted_file.external_dependencies = cached_file.external_dependencies;
    extracted_file.resolved_imports =
        serde_json::from_slice(&cached_resolutions.resolved_imports_json)
            .map_err(|err| miette::miette!("{err}"))?;
    extracted_file.resolved_reexports =
        serde_json::from_slice(&cached_resolutions.resolved_reexports_json)
            .map_err(|err| miette::miette!("{err}"))?;
    counters.hits += 1;
    Ok(true)
}

fn detect_all_entrypoints(
    discovery: &DiscoveryResult,
    config: &EntrypointsConfig,
    frameworks_config: Option<&pruneguard_config::FrameworksConfig>,
    packs: &[Box<dyn pruneguard_frameworks::FrameworkPack>],
    exclude_matcher: Option<&GlobSet>,
    scan_roots: &[PathBuf],
    file_inventory: &[PathBuf],
) -> Vec<EntrypointSeed> {
    let mut entrypoints = Vec::new();
    let ws_count = discovery.workspaces.len();
    let is_monorepo = ws_count > 1;

    for workspace in discovery.workspaces.values() {
        // In a monorepo, skip expensive framework pack detection for the root
        // workspace since child workspaces handle their own framework
        // entrypoints.  This avoids repeated full-tree filesystem traversals
        // from `pack.entrypoints()` on the monorepo root.
        let effective_packs: &[Box<dyn pruneguard_frameworks::FrameworkPack>] =
            if is_monorepo && workspace.root == discovery.project_root { &[] } else { packs };
        let mut workspace_entrypoints = detect_entrypoints(
            Some(workspace.name.as_str()),
            &workspace.root,
            &workspace.manifest,
            config,
            frameworks_config,
            effective_packs,
            Some(file_inventory),
        );

        workspace_entrypoints.retain(|entrypoint| {
            if !scan_roots.is_empty()
                && !scan_roots.iter().any(|root| entrypoint.path.starts_with(root))
            {
                return false;
            }

            let relative =
                entrypoint.path.strip_prefix(&discovery.project_root).unwrap_or(&entrypoint.path);
            if exclude_matcher.is_some_and(|matcher| matcher.is_match(relative)) {
                return false;
            }

            // Framework-contributed entrypoints (e.g. vitest config, storybook
            // stories) bypass the include_tests / include_stories filters — they
            // were explicitly added by the framework pack precisely because the
            // framework owns those files.
            if entrypoint.kind != SeedKind::FrameworkPack {
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
            }

            true
        });

        entrypoints.extend(workspace_entrypoints);
    }

    entrypoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.source.cmp(&b.source)));
    entrypoints.dedup_by(|left, right| left.path == right.path && left.profile == right.profile);
    entrypoints
}

fn filter_entrypoints_by_profile(
    entrypoints: &mut Vec<EntrypointSeed>,
    profile: EntrypointProfile,
) {
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
    config: &PruneguardConfig,
) -> bool {
    // Explicit config and framework-contributed entrypoints are always kept.
    if seed.kind == SeedKind::ExplicitConfig || seed.kind == SeedKind::FrameworkPack {
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
            name: workspace.manifest.name.clone().unwrap_or_else(|| workspace.name.clone()),
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
                FileKind::Source => pruneguard_report::FileKind::Source,
                FileKind::Test => pruneguard_report::FileKind::Test,
                FileKind::Story => pruneguard_report::FileKind::Story,
                FileKind::Config => pruneguard_report::FileKind::Config,
                FileKind::Generated => pruneguard_report::FileKind::Generated,
                FileKind::BuildOutput => pruneguard_report::FileKind::BuildOutput,
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
            framework: None,
            reason: None,
            heuristic: None,
        });
    }

    entrypoints.sort_by(|a, b| {
        a.path.cmp(&b.path).then(a.kind.cmp(&b.kind)).then(a.source.cmp(&b.source))
    });
    entrypoints
}

fn seed_public_exports_with_config(
    symbol_graph: &mut SymbolGraph,
    entrypoint_seeds: &[EntrypointSeed],
    file_nodes: &FxHashMap<PathBuf, (crate::FileId, petgraph::graph::NodeIndex)>,
    include_entry_exports: bool,
) {
    for seed in entrypoint_seeds {
        let Some((file_id, _)) = file_nodes.get(&seed.path) else {
            continue;
        };

        // Decide whether this seed's exports should be marked live.
        //
        // When `include_entry_exports` is true, *no* entrypoint kind has its
        // exports automatically marked live — the analyzer checks every export
        // for actual usage across the graph (matching knip's behaviour).
        //
        // When `include_entry_exports` is false (the default), all entrypoint
        // exports are marked live so they are never reported as unused.
        let should_mark_live = !include_entry_exports;

        if should_mark_live {
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

#[allow(clippy::too_many_lines)]
fn add_symbol_edges(
    symbol_graph: &mut SymbolGraph,
    importer_id: crate::FileId,
    facts: &pruneguard_extract::FileFacts,
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
            symbol_graph.add_import(importer_id, *source_id, name.imported.clone(), import.is_type);
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
            if reexport.names.is_empty() {
                // `export * from './mod'` — true star re-export.
                symbol_graph.add_reexport(
                    importer_id,
                    *source_id,
                    CompactString::new("*"),
                    CompactString::new("*"),
                    true,
                    reexport.is_type,
                );
            } else {
                // `export * as Name from './mod'` — namespace re-export.
                // The names vec has a single entry: {original: "*", exported: "Name"}.
                for name in &reexport.names {
                    symbol_graph.add_reexport(
                        importer_id,
                        *source_id,
                        name.original.clone(),
                        name.exported.clone(),
                        true,
                        reexport.is_type,
                    );
                    // Also register the namespace alias as an export of the
                    // re-exporting file so import demand can flow to it.
                    symbol_graph.add_export(importer_id, name.exported.clone(), reexport.is_type);
                }
            }
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

    // Register member exports (class methods, enum variants, namespace members).
    for member in &facts.member_exports {
        let member_kind = match member.member_kind {
            MemberKind::Method => MemberNodeKind::Method,
            MemberKind::Property => MemberNodeKind::Property,
            MemberKind::EnumVariant => MemberNodeKind::EnumVariant,
            MemberKind::NamespaceMember => MemberNodeKind::NamespaceMember,
            MemberKind::StaticMethod => MemberNodeKind::StaticMethod,
            MemberKind::StaticProperty => MemberNodeKind::StaticProperty,
            MemberKind::Getter => MemberNodeKind::Getter,
            MemberKind::Setter => MemberNodeKind::Setter,
        };
        symbol_graph.add_member_export(
            importer_id,
            member.parent_name.clone(),
            member.member_name.clone(),
            member_kind,
            member.is_public_tagged,
        );
    }

    // Register same-file references to exports.
    for same_ref in &facts.same_file_refs {
        symbol_graph.add_same_file_ref(importer_id, same_ref.export_name.clone(), same_ref.line);
    }

    // Register member access patterns (e.g., Color.Red → member ref on Color's Red member).
    // Build a map from local import name → (source_id, imported_name) for lookup.
    let import_map: FxHashMap<&str, (crate::FileId, &str)> = facts
        .imports
        .iter()
        .zip(resolved_imports.iter())
        .filter_map(|(import, edge)| {
            let source_path = edge.to_file.as_ref()?;
            let (source_id, _) = file_nodes.get(source_path)?;
            Some(
                import
                    .names
                    .iter()
                    .map(move |name| (name.local.as_str(), (*source_id, name.imported.as_str()))),
            )
        })
        .flatten()
        .collect();

    for access in &facts.member_accesses {
        if let Some(&(source_id, export_name)) = import_map.get(access.object_name.as_str()) {
            let access_kind =
                if access.is_write { MemberAccessKind::Write } else { MemberAccessKind::Read };
            if export_name == "*" {
                // Namespace import: `import * as NS from './mod'` + `NS.member`.
                // Create a direct import edge for the specific member so the
                // unused_exports analyzer knows the export is consumed.
                symbol_graph.add_import(
                    importer_id,
                    source_id,
                    CompactString::new(&access.member_name),
                    false,
                );
            } else {
                // Named import: `import { Color } from './color'` + `Color.Red`.
                symbol_graph.add_member_ref(
                    importer_id,
                    source_id,
                    CompactString::new(export_name),
                    CompactString::new(&access.member_name),
                    false,
                    access_kind,
                );
            }
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
        ResolvedEdgeKind::RequireResolve => ModuleEdge::RequireResolve,
        ResolvedEdgeKind::ImportMetaGlob => ModuleEdge::ImportMetaGlob,
        ResolvedEdgeKind::JsDocImport => ModuleEdge::JsDocImport,
        ResolvedEdgeKind::TripleSlashFile => ModuleEdge::TripleSlashFile,
        ResolvedEdgeKind::TripleSlashTypes => ModuleEdge::TripleSlashTypes,
        ResolvedEdgeKind::ImportMetaResolve => ModuleEdge::ImportMetaResolve,
        ResolvedEdgeKind::RequireContext => ModuleEdge::RequireContext,
        ResolvedEdgeKind::UrlConstructor => ModuleEdge::UrlConstructor,
        ResolvedEdgeKind::ImportEquals => ModuleEdge::ImportEquals,
    }
}

/// Hard cap on expanded edges from a single glob/context pattern to prevent runaway expansion.
const MAX_EXPANDED_EDGES: usize = 500;

/// Expand an `import.meta.glob` wildcard pattern against the tracked source inventory.
///
/// For each file in `repo_files` that matches the glob, a `ResolvedEdgeKind::ImportMetaGlob`
/// edge is added. Negation patterns (starting with `!`) in `negations` are used to filter
/// out matched files. When the result set is truncated at `MAX_EXPANDED_EDGES`, a diagnostic
/// is emitted to note the truncation.
fn expand_glob_into_edges(
    extracted_file: &mut ExtractedFile,
    pattern: &str,
    negations: &[&str],
    line: u32,
    resolver: &ModuleResolver,
    repo_files: &RepoFileIndex,
) {
    // Skip negation patterns — they exclude rather than include.
    if pattern.starts_with('!') {
        return;
    }

    let source_dir = extracted_file.file.path.parent().unwrap_or(&extracted_file.file.path);
    // Normalize `./` prefix: globset treats `./foo` literally, but import.meta.glob
    // uses `./` to mean "relative to current file's directory".
    let normalized_pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let Ok(glob) = Glob::new(normalized_pattern) else {
        return;
    };
    let matcher = glob.compile_matcher();

    // Build a GlobSet from negation patterns for efficient filtering.
    let negation_set = if negations.is_empty() {
        None
    } else {
        let mut builder = GlobSetBuilder::new();
        for neg in negations {
            // Strip the leading `!` and `./` to get the raw glob pattern.
            let raw = neg.strip_prefix('!').unwrap_or(neg);
            let raw = raw.strip_prefix("./").unwrap_or(raw);
            if let Ok(g) = Glob::new(raw) {
                builder.add(g);
            }
        }
        builder.build().ok()
    };

    let mut count = 0;
    let mut truncated = false;
    for file_path in repo_files.iter_original() {
        if count >= MAX_EXPANDED_EDGES {
            truncated = true;
            break;
        }
        // Match against the relative path from the source file's directory.
        let Ok(relative) = file_path.strip_prefix(source_dir) else {
            continue;
        };
        if !matcher.is_match(relative) {
            continue;
        }
        // Apply negation filter: if the file matches any negation glob, skip it.
        if let Some(ref neg_set) = negation_set
            && neg_set.is_match(relative)
        {
            continue;
        }
        extracted_file.resolved_imports.push(resolve_edge(
            resolver,
            &extracted_file.file.path,
            &file_path.to_string_lossy(),
            ResolvedEdgeKind::ImportMetaGlob,
            line,
            repo_files,
        ));
        count += 1;
    }

    if truncated {
        tracing::warn!(
            file = %extracted_file.file.path.display(),
            pattern,
            cap = MAX_EXPANDED_EDGES,
            "import.meta.glob expansion truncated at {MAX_EXPANDED_EDGES} edges; \
             some matching files may not be tracked as dependencies",
        );
        extracted_file.parse_diagnostics.push(format!(
            "import.meta.glob pattern `{pattern}` expanded to >{MAX_EXPANDED_EDGES} files; \
             results truncated (confidence lowered)",
        ));
    }
}

/// Expand a `require.context(directory, recursive, regex_filter)` against the tracked source
/// inventory. When `regex_filter` is provided, only files whose path (relative to the context
/// directory) matches the regex are included.
fn expand_require_context_into_edges(
    extracted_file: &mut ExtractedFile,
    directory: &str,
    recursive: bool,
    regex_filter: Option<&str>,
    line: u32,
    resolver: &ModuleResolver,
    repo_files: &RepoFileIndex,
) {
    let source_dir = extracted_file.file.path.parent().unwrap_or(&extracted_file.file.path);
    let context_dir = if Path::new(directory).is_absolute() {
        PathBuf::from(directory)
    } else {
        let stripped = directory.strip_prefix("./").unwrap_or(directory);
        source_dir.join(stripped)
    };

    // Pre-compile the regex filter if one was provided.
    let compiled_regex = regex_filter
        .and_then(|pat| globset::Glob::new(&format!("*{pat}*")).ok().map(|g| g.compile_matcher()));
    // Also attempt a true regex compilation for precise JS-regex semantics.
    let true_regex = regex_filter.and_then(|pat| regex_lite::Regex::new(pat).ok());

    let mut count = 0;
    let mut truncated = false;
    for file_path in repo_files.iter_original() {
        if count >= MAX_EXPANDED_EDGES {
            truncated = true;
            break;
        }
        if !file_path.starts_with(&context_dir) {
            continue;
        }
        let Ok(relative) = file_path.strip_prefix(&context_dir) else {
            continue;
        };
        // Non-recursive: only direct children.
        if !recursive && relative.components().count() > 1 {
            continue;
        }
        // Apply regex filter against the relative path.
        if let Some(ref re) = true_regex {
            let rel_str = relative.to_string_lossy();
            if !re.is_match(&rel_str) {
                continue;
            }
        } else if let Some(ref glob_matcher) = compiled_regex
            && !glob_matcher.is_match(relative)
        {
            continue;
        }
        extracted_file.resolved_imports.push(resolve_edge(
            resolver,
            &extracted_file.file.path,
            &file_path.to_string_lossy(),
            ResolvedEdgeKind::RequireContext,
            line,
            repo_files,
        ));
        count += 1;
    }

    if truncated {
        tracing::warn!(
            file = %extracted_file.file.path.display(),
            directory,
            cap = MAX_EXPANDED_EDGES,
            "require.context expansion truncated at {MAX_EXPANDED_EDGES} edges; \
             some matching files may not be tracked as dependencies",
        );
        extracted_file.parse_diagnostics.push(format!(
            "require.context(`{directory}`) expanded to >{MAX_EXPANDED_EDGES} files; \
             results truncated (confidence lowered)",
        ));
    }
}

/// Inject entrypoints discovered from framework config adapters.
#[allow(clippy::too_many_lines)]
fn inject_config_entrypoints(
    entrypoint_seeds: &mut Vec<EntrypointSeed>,
    config_inputs: &pruneguard_config_readers::ConfigInputs,
    discovery: &DiscoveryResult,
    file_inventory: &[PathBuf],
) {
    let project_root = &discovery.project_root;
    let existing: FxHashSet<PathBuf> = entrypoint_seeds.iter().map(|s| s.path.clone()).collect();

    // General entrypoints from config.
    for entry_path in &config_inputs.entrypoints {
        let abs = if entry_path.is_absolute() {
            entry_path.clone()
        } else {
            project_root.join(entry_path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::Runtime,
                profile: EntrypointProfile::Both,
                workspace: None,
                source: config_inputs.framework.as_deref().unwrap_or("config-adapter").to_string(),
            });
        }
    }

    // Runtime entrypoints.
    for entry_path in &config_inputs.runtime_entrypoints {
        let abs = if entry_path.is_absolute() {
            entry_path.clone()
        } else {
            project_root.join(entry_path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::Runtime,
                profile: EntrypointProfile::Production,
                workspace: None,
                source: config_inputs.framework.as_deref().unwrap_or("config-adapter").to_string(),
            });
        }
    }

    // Production entrypoints.
    for entry_path in &config_inputs.production_entrypoints {
        let abs = if entry_path.is_absolute() {
            entry_path.clone()
        } else {
            project_root.join(entry_path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::FrameworkConvention,
                profile: EntrypointProfile::Production,
                workspace: None,
                source: config_inputs.framework.as_deref().unwrap_or("config-adapter").to_string(),
            });
        }
    }

    // Development entrypoints.
    for entry_path in &config_inputs.development_entrypoints {
        let abs = if entry_path.is_absolute() {
            entry_path.clone()
        } else {
            project_root.join(entry_path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::Tooling,
                profile: EntrypointProfile::Development,
                workspace: None,
                source: config_inputs.framework.as_deref().unwrap_or("config-adapter").to_string(),
            });
        }
    }

    // Setup files.
    for entry_path in config_inputs.setup_files.iter().chain(&config_inputs.global_setup_files) {
        let abs = if entry_path.is_absolute() {
            entry_path.clone()
        } else {
            project_root.join(entry_path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::Tooling,
                profile: EntrypointProfile::Development,
                workspace: None,
                source: config_inputs.framework.as_deref().unwrap_or("config-adapter").to_string(),
            });
        }
    }

    // Story entrypoints.
    for entry_path in &config_inputs.story_entrypoints {
        let abs = if entry_path.is_absolute() {
            entry_path.clone()
        } else {
            project_root.join(entry_path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::Story,
                profile: EntrypointProfile::Development,
                workspace: None,
                source: config_inputs.framework.as_deref().unwrap_or("config-adapter").to_string(),
            });
        }
    }

    // Generated entrypoints (e.g. from Nuxt .d.ts files).
    for generated in &config_inputs.generated_entrypoints {
        let abs = if generated.path.is_absolute() {
            generated.path.clone()
        } else {
            project_root.join(&generated.path)
        };
        if !existing.contains(&abs) && abs.exists() {
            entrypoint_seeds.push(EntrypointSeed {
                path: abs,
                kind: SeedKind::FrameworkPack,
                surface_kind: EntrypointSurfaceKind::FrameworkConvention,
                profile: EntrypointProfile::Both,
                workspace: None,
                source: format!("generated:{}", generated.kind),
            });
        }
    }

    // Test file patterns (e.g. "**/*.test.*", "**/*.spec.*") from framework
    // config adapters (vitest, jest, playwright).  These make individual test
    // files into Development entrypoints so their imports count as used.
    //
    // When no explicit test patterns are configured but a test runner
    // (vitest, jest) is detected as a dependency, fall back to conventional
    // test file patterns so test files are not falsely flagged as unused.
    {
        let mut effective_test_patterns = config_inputs.test_patterns.clone();
        if effective_test_patterns.is_empty() {
            // Check if any workspace has vitest or jest as a dependency.
            let has_test_runner = discovery.workspaces.values().any(|ws| {
                let has_dep = |name: &str| -> bool {
                    ws.manifest.dependencies.as_ref().is_some_and(|deps| deps.contains_key(name))
                        || ws
                            .manifest
                            .dev_dependencies
                            .as_ref()
                            .is_some_and(|deps| deps.contains_key(name))
                };
                has_dep("vitest") || has_dep("jest")
            });
            if has_test_runner {
                effective_test_patterns = vec![
                    "**/*.test.*".to_string(),
                    "**/*.spec.*".to_string(),
                    "**/__tests__/**/*.{ts,tsx,js,jsx}".to_string(),
                    "**/tests/**/*.test.*".to_string(),
                    "**/tests/**/*.spec.*".to_string(),
                ];
            }
        }

        let mut test_globs = Vec::new();
        for pattern in &effective_test_patterns {
            if let Ok(glob) = Glob::new(pattern) {
                test_globs.push(glob.compile_matcher());
            }
        }
        if !test_globs.is_empty() {
            for file_path in file_inventory {
                if existing.contains(file_path) {
                    continue;
                }
                for workspace in discovery.workspaces.values() {
                    if !file_path.starts_with(&workspace.root) {
                        continue;
                    }
                    let relative = file_path.strip_prefix(&workspace.root).unwrap_or(file_path);
                    if test_globs.iter().any(|g| g.is_match(relative))
                        && pruneguard_fs::is_tracked_source(file_path)
                    {
                        entrypoint_seeds.push(EntrypointSeed {
                            path: file_path.clone(),
                            kind: SeedKind::FrameworkPack,
                            surface_kind: EntrypointSurfaceKind::Test,
                            profile: EntrypointProfile::Development,
                            workspace: Some(workspace.name.clone()),
                            source: config_inputs
                                .framework
                                .as_deref()
                                .unwrap_or("test-pattern")
                                .to_string(),
                        });
                    }
                    break;
                }
            }
        }
    }

    // Test support files (setupTests, global.setup, global.teardown, etc.)
    // loaded by test runner configuration, not via imports.
    {
        let test_setup_patterns = [
            "**/setupTests.*",
            "**/setup.ts",
            "**/setup.tsx",
            "**/setup.js",
            "**/global.setup.*",
            "**/global.teardown.*",
            "**/setup/global.setup.*",
            "**/setup/global.teardown.*",
        ];
        let mut test_setup_globs = Vec::new();
        for pattern in &test_setup_patterns {
            if let Ok(glob) = Glob::new(pattern) {
                test_setup_globs.push(glob.compile_matcher());
            }
        }
        if !test_setup_globs.is_empty() {
            for file_path in file_inventory {
                if existing.contains(file_path) {
                    continue;
                }
                for workspace in discovery.workspaces.values() {
                    if !file_path.starts_with(&workspace.root) {
                        continue;
                    }
                    let relative = file_path.strip_prefix(&workspace.root).unwrap_or(file_path);
                    if test_setup_globs.iter().any(|g| g.is_match(relative))
                        && pruneguard_fs::is_tracked_source(file_path)
                    {
                        entrypoint_seeds.push(EntrypointSeed {
                            path: file_path.clone(),
                            kind: SeedKind::FrameworkPack,
                            surface_kind: EntrypointSurfaceKind::Tooling,
                            profile: EntrypointProfile::Development,
                            workspace: Some(workspace.name.clone()),
                            source: "test-setup".to_string(),
                        });
                    }
                    break;
                }
            }
        }
    }

    // .dev.{ts,tsx,js,jsx} conditional build variants.
    // When a `.dev.` variant exists alongside a non-dev version (e.g.,
    // `index.dev.tsx` next to `index.tsx`), the dev file is treated as a
    // Development entrypoint.
    {
        let dev_variant_pattern = "*.dev.{ts,tsx,js,jsx}";
        if let Ok(glob) = Glob::new(dev_variant_pattern) {
            let matcher = glob.compile_matcher();
            for file_path in file_inventory {
                if existing.contains(file_path) {
                    continue;
                }
                let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !matcher.is_match(file_name) {
                    continue;
                }
                // Derive the non-dev counterpart: `foo.dev.tsx` → `foo.tsx`
                if let Some(stem_before_dev) = file_name
                    .strip_suffix(".dev.ts")
                    .or_else(|| file_name.strip_suffix(".dev.tsx"))
                    .or_else(|| file_name.strip_suffix(".dev.js"))
                    .or_else(|| file_name.strip_suffix(".dev.jsx"))
                {
                    let ext = &file_name[stem_before_dev.len() + ".dev".len()..];
                    let non_dev_name = format!("{stem_before_dev}{ext}");
                    let non_dev_path = file_path.with_file_name(&non_dev_name);
                    if file_inventory.contains(&non_dev_path) {
                        entrypoint_seeds.push(EntrypointSeed {
                            path: file_path.clone(),
                            kind: SeedKind::FrameworkPack,
                            surface_kind: EntrypointSurfaceKind::Tooling,
                            profile: EntrypointProfile::Development,
                            workspace: None,
                            source: "dev-variant".to_string(),
                        });
                    }
                }
            }
        }
    }

    // Route entry globs (e.g. TanStack Router, React Router, Vike, Qwik City).
    // Use the pre-discovered file inventory instead of filesystem traversal.
    if !config_inputs.route_entry_globs.is_empty() {
        // Pre-group files by workspace for efficient lookup.
        let mut files_by_workspace: FxHashMap<&Path, Vec<(&Path, &PathBuf)>> = FxHashMap::default();
        for workspace in discovery.workspaces.values() {
            files_by_workspace.insert(&workspace.root, Vec::new());
        }
        for file_path in file_inventory {
            for workspace in discovery.workspaces.values() {
                if file_path.starts_with(&workspace.root) {
                    let relative = file_path.strip_prefix(&workspace.root).unwrap_or(file_path);
                    files_by_workspace
                        .entry(&workspace.root)
                        .or_default()
                        .push((relative, file_path));
                    break; // Each file belongs to at most one workspace (most specific).
                }
            }
        }

        for glob_entry in &config_inputs.route_entry_globs {
            if let Ok(glob) = Glob::new(&glob_entry.pattern) {
                let matcher = glob.compile_matcher();
                for workspace in discovery.workspaces.values() {
                    if let Some(ws_files) = files_by_workspace.get(workspace.root.as_path()) {
                        for (relative, file_path) in ws_files {
                            if matcher.is_match(relative) && !existing.contains(*file_path) {
                                entrypoint_seeds.push(EntrypointSeed {
                                    path: (*file_path).clone(),
                                    kind: SeedKind::FrameworkPack,
                                    surface_kind: EntrypointSurfaceKind::FrameworkConvention,
                                    profile: EntrypointProfile::Production,
                                    workspace: Some(workspace.name.clone()),
                                    source: format!("route-glob:{}", glob_entry.framework),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

fn normalize_scan_roots(cwd: &Path, scan_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = scan_paths
        .iter()
        .map(|path| if path.is_absolute() { path.clone() } else { cwd.join(path) })
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
    path.strip_prefix(project_root).unwrap_or(path).to_string_lossy().to_string()
}

fn compute_tsconfig_hash(project_root: &Path, files: &[pruneguard_fs::FileRecord]) -> u64 {
    let mut tsconfig_paths = files
        .iter()
        .map(|file| &file.relative_path)
        .filter(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
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
