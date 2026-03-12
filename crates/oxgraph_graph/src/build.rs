use std::path::{Path, PathBuf};
use std::time::Instant;

use compact_str::CompactString;
use globset::{Glob, GlobSet, GlobSetBuilder};
use miette::{IntoDiagnostic, Result};
use oxgraph_config::{EntrypointsConfig, OxgraphConfig};
use oxgraph_discovery::{DiscoveryResult, discover};
use oxgraph_entrypoints::{EntrypointKind as SeedKind, EntrypointProfile, EntrypointSeed, detect_entrypoints};
use oxgraph_extract::{ExtractedFile, extract_file_facts};
use oxgraph_frameworks::built_in_packs;
use oxgraph_fs::{FileKind, has_js_ts_extension};
use oxgraph_report::{EntrypointInfo, FileInfo, Inventories, PackageInfo, Stats, WorkspaceInfo};
use oxgraph_resolver::{ModuleResolver, ResolvedEdge, ResolvedEdgeKind, dependency_name};
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
pub fn build_graph(
    cwd: &Path,
    config: &OxgraphConfig,
    scan_paths: &[PathBuf],
    profile: EntrypointProfile,
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

    let resolver = ModuleResolver::new(&config.resolver);
    let mut extracted_files = files.into_iter().map(ExtractedFile::new).collect::<Vec<_>>();
    let repo_files = extracted_files
        .iter()
        .map(|file| file.file.path.clone())
        .collect::<FxHashSet<_>>();

    for extracted_file in &mut extracted_files {
        populate_extracted_file(extracted_file, &resolver, &repo_files)?;
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

    let stats = Stats {
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        files_parsed: extracted_files.iter().filter(|file| file.facts.is_some()).count(),
        files_cached: 0,
        graph_nodes: module_graph.node_count(),
        graph_edges: module_graph.edge_count(),
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

fn populate_extracted_file(
    extracted_file: &mut ExtractedFile,
    resolver: &ModuleResolver,
    repo_files: &FxHashSet<PathBuf>,
) -> Result<()> {
    if !has_js_ts_extension(&extracted_file.file.path) {
        return Ok(());
    }

    let source = std::fs::read_to_string(&extracted_file.file.path).into_diagnostic()?;
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
            via_exports: module.via_exports,
            line: Some(line),
        },
        Ok(module) => ResolvedEdge {
            from: from.to_path_buf(),
            specifier: specifier.to_string(),
            to_file: None,
            to_dependency: dependency_name(specifier).or_else(|| module.path.file_name().map(|name| name.to_string_lossy().to_string())),
            kind,
            via_exports: module.via_exports,
            line: Some(line),
        },
        Err(_) => ResolvedEdge {
            from: from.to_path_buf(),
            specifier: specifier.to_string(),
            to_file: None,
            to_dependency: dependency_name(specifier),
            kind,
            via_exports: false,
            line: Some(line),
        },
    }
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
        });
    }

    entrypoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.kind.cmp(&b.kind)));
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
