use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use miette::IntoDiagnostic;
use oxgraph_cache::AnalysisCache;
use oxgraph_config::OxgraphConfig;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_git::{ChangedScope, collect_changed_scope};
use oxgraph_graph::{
    BuildOptions, GraphBuildResult, ModuleEdge, ModuleNode, build_graph,
    build_graph_with_options,
};
use oxgraph_report::{AnalysisReport, ExplainReport, Finding, ImpactReport, ProofNode, Summary};

#[cfg(feature = "napi")]
use napi_derive::napi;

#[cfg(feature = "napi")]
mod migrate;

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub config_dir: Option<PathBuf>,
    pub changed_since: Option<String>,
    pub no_cache: bool,
}

#[derive(Debug)]
pub struct ScanExecution {
    pub report: AnalysisReport,
    pub build: GraphBuildResult,
}

#[derive(Debug, Clone)]
struct BaselineSet {
    source_path: PathBuf,
    tool_version: String,
    profile: String,
    finding_ids: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct AffectedScope {
    files: BTreeSet<String>,
    packages: BTreeSet<String>,
    entrypoints: BTreeSet<String>,
    incomplete: bool,
}

/// Run a full scan and return the analysis report.
pub fn scan(
    cwd: &Path,
    config: &OxgraphConfig,
    paths: &[PathBuf],
    profile: EntrypointProfile,
) -> miette::Result<AnalysisReport> {
    Ok(scan_with_options(cwd, config, paths, profile, &ScanOptions::default())?.report)
}

pub fn scan_with_options(
    cwd: &Path,
    config: &OxgraphConfig,
    paths: &[PathBuf],
    profile: EntrypointProfile,
    options: &ScanOptions,
) -> miette::Result<ScanExecution> {
    let scan_roots = normalize_scan_roots(cwd, paths);
    let discovery_cwd = scan_roots.first().map_or_else(|| cwd.to_path_buf(), Clone::clone);
    let discovery = oxgraph_discovery::discover(&discovery_cwd, config)?;
    let cache = if options.no_cache {
        None
    } else {
        Some(AnalysisCache::open(&discovery.project_root).map_err(|err| miette::miette!("{err}"))?)
    };
    let build = build_graph_with_options(
        cwd,
        config,
        paths,
        profile,
        BuildOptions { cache: cache.as_ref() },
    )?;
    let findings = oxgraph_analyzers::run_analyzers(&build, config, profile);
    let mut report = report_from_build(cwd, &build, findings, profile);

    if let Some(reference) = &options.changed_since {
        let mut changed_scope =
            collect_changed_scope(&build.discovery.project_root, reference, &scan_roots)?;
        recover_deleted_paths(cache.as_ref(), &mut changed_scope);
        apply_changed_scope(&mut report, &build, &changed_scope, cache.as_ref(), profile);
    }

    if let Some(baseline) = load_baseline(
        options.config_dir.as_deref(),
        &build.discovery.project_root,
    )? {
        apply_baseline(&mut report, &baseline);
    }

    refresh_summary(&mut report);

    Ok(ScanExecution { report, build })
}

/// Compute the blast radius for a target.
pub fn impact(
    cwd: &Path,
    config: &OxgraphConfig,
    target: &str,
    profile: EntrypointProfile,
) -> miette::Result<ImpactReport> {
    let build = build_graph(cwd, config, &[], profile)?;
    let target_file = build
        .find_file(target)
        .ok_or_else(|| miette::miette!("target `{target}` did not match a tracked file"))?;
    let file_id = build
        .module_graph
        .file_id(&target_file.file.path.to_string_lossy())
        .ok_or_else(|| miette::miette!("target `{target}` is not present in the module graph"))?;

    let analysis = oxgraph_analyzers::impact::analyze(&build, file_id, profile);
    let proofs = build
        .module_graph
        .shortest_path_to_file(file_id, profile)
        .map(|path| {
            path.iter()
                .map(|index| node_label(&build, *index))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ImpactReport {
        target: target_file.file.relative_path.to_string_lossy().to_string(),
        affected_entrypoints: analysis.affected_entrypoints,
        affected_packages: analysis.affected_packages,
        affected_files: analysis.affected_files,
        evidence: if proofs.is_empty() {
            Vec::new()
        } else {
            vec![oxgraph_report::Evidence {
                kind: "path".to_string(),
                file: Some(target_file.file.relative_path.to_string_lossy().to_string()),
                line: None,
                description: format!("One entrypoint path to the target: {}", proofs.join(" -> ")),
            }]
        },
    })
}

/// Explain a finding or path.
pub fn explain(
    cwd: &Path,
    config: &OxgraphConfig,
    query: &str,
    profile: EntrypointProfile,
) -> miette::Result<ExplainReport> {
    let build = build_graph(cwd, config, &[], profile)?;
    let findings = oxgraph_analyzers::run_analyzers(&build, config, profile);
    let related_findings = findings
        .iter()
        .filter(|finding| finding.id == query || finding.subject == query)
        .cloned()
        .collect::<Vec<_>>();

    let target_query = related_findings
        .first()
        .map_or(query, |finding| finding.subject.as_str());
    let matched_file = build.find_file(target_query).or_else(|| {
        target_query
            .split_once('#')
            .and_then(|(path, _)| build.find_file(path))
    });

    let matched_node = matched_file
        .map(|file| file.file.relative_path.to_string_lossy().to_string())
        .or_else(|| related_findings.first().map(|finding| finding.subject.clone()));

    let proofs = matched_file
        .and_then(|file| {
                build.module_graph.file_id(&file.file.path.to_string_lossy()).and_then(|file_id| {
                    build.module_graph.shortest_path_to_file(file_id, profile).map(|path| {
                        build_proof_tree(
                            path.iter().map(|index| node_label(&build, *index)).collect::<Vec<_>>(),
                        )
                    })
                })
            })
        .into_iter()
        .collect::<Vec<_>>();

    Ok(ExplainReport { query: query.to_string(), matched_node, proofs, related_findings })
}

/// Debug: list all detected entrypoints.
pub fn debug_entrypoints(
    cwd: &Path,
    config: &OxgraphConfig,
    profile: EntrypointProfile,
) -> miette::Result<Vec<String>> {
    let build = build_graph(cwd, config, &[], profile)?;
    Ok(build.entrypoint_seeds.iter().map(ToString::to_string).collect())
}

/// Debug: resolve a specifier from a file.
pub fn debug_resolve(cwd: &Path, config: &OxgraphConfig, specifier: &str, from: &Path) -> String {
    oxgraph_resolver::debug_resolve(cwd, &config.resolver, specifier, from)
}

fn report_from_build(
    cwd: &Path,
    build: &GraphBuildResult,
    findings: Vec<Finding>,
    profile: EntrypointProfile,
) -> AnalysisReport {
    let (errors, warnings, infos) = finding_summary(&findings);

    AnalysisReport {
        version: 1,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        cwd: cwd.to_string_lossy().to_string(),
        profile: profile.as_str().to_string(),
        summary: Summary {
            total_files: build.inventories.files.len(),
            total_packages: build.inventories.packages.len(),
            total_workspaces: build.inventories.workspaces.len(),
            total_exports: build.symbol_graph.exports.len(),
            total_findings: findings.len(),
            errors,
            warnings,
            infos,
        },
        inventories: build.inventories.clone(),
        findings,
        entrypoints: build.entrypoints.clone(),
        stats: build.stats.clone(),
    }
}

fn load_baseline(
    config_dir: Option<&Path>,
    project_root: &Path,
) -> miette::Result<Option<BaselineSet>> {
    let mut candidates = Vec::new();
    if let Some(config_dir) = config_dir {
        candidates.push(config_dir.join("baseline.json"));
    }
    candidates.push(project_root.join("baseline.json"));

    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&candidate).into_diagnostic()?;
        let report: AnalysisReport = serde_json::from_str(&content).into_diagnostic()?;
        let finding_ids = report.findings.into_iter().map(|finding| finding.id).collect();
        return Ok(Some(BaselineSet {
            source_path: candidate,
            tool_version: report.tool_version,
            profile: report.profile,
            finding_ids,
        }));
    }

    Ok(None)
}

fn apply_baseline(report: &mut AnalysisReport, baseline: &BaselineSet) {
    let original_len = report.findings.len();
    report.findings.retain(|finding| !baseline.finding_ids.contains(&finding.id));
    report.stats.baseline_applied = true;
    report.stats.baseline_profile_mismatch = baseline.profile != report.profile;
    report.stats.suppressed_findings = original_len.saturating_sub(report.findings.len());
    report.stats.new_findings = report.findings.len();

    if report.stats.baseline_profile_mismatch {
        report.findings.sort_by(|left, right| left.id.cmp(&right.id));
        if let Some(first) = report.findings.first_mut() {
            first.evidence.push(oxgraph_report::Evidence {
                kind: "path".to_string(),
                file: Some(baseline.source_path.to_string_lossy().to_string()),
                line: None,
                description: format!(
                    "baseline profile `{}` differs from current profile `{}` (tool {}).",
                    baseline.profile, report.profile, baseline.tool_version
                ),
            });
        }
    }
}

fn apply_changed_scope(
    report: &mut AnalysisReport,
    build: &GraphBuildResult,
    changed_scope: &ChangedScope,
    cache: Option<&AnalysisCache>,
    profile: EntrypointProfile,
) {
    report.stats.changed_files = changed_scope.changed_paths().len();
    let affected = compute_affected_scope(build, changed_scope, cache, profile);
    report.stats.affected_scope_incomplete = affected.incomplete;
    report.stats.affected_files = affected.files.len();
    report.stats.affected_packages = affected.packages.len();
    report.stats.affected_entrypoints = affected.entrypoints.len();

    if affected.incomplete {
        return;
    }

    report
        .findings
        .retain(|finding| finding_in_affected_scope(finding, &affected));
}

fn compute_affected_scope(
    build: &GraphBuildResult,
    changed_scope: &ChangedScope,
    cache: Option<&AnalysisCache>,
    profile: EntrypointProfile,
) -> AffectedScope {
    let mut affected = AffectedScope {
        incomplete: !changed_scope.unrecoverable_deleted.is_empty(),
        ..AffectedScope::default()
    };

    let mut changed_paths = changed_scope.added.clone();
    changed_paths.extend(changed_scope.modified.iter().cloned());
    changed_paths.extend(changed_scope.renamed.iter().map(|rename| rename.to.clone()));

    for relative_path in &changed_paths {
        let query = relative_path.to_string_lossy();
        let Some(file) = build.find_file(&query) else {
            continue;
        };
        affected
            .files
            .insert(file.file.relative_path.to_string_lossy().to_string());
        if let Some(package) = &file.file.package {
            affected.packages.insert(package.clone());
        }
        if let Some(file_id) = build.module_graph.file_id(&file.file.path.to_string_lossy()) {
            add_reverse_reachable_scope(build, file_id, profile, &mut affected);
        }
    }

    if let Some(cache) = cache {
        for relative_path in &changed_scope.recoverable_deleted {
            if let Ok(Some(entry)) = cache.lookup_path_index(relative_path) {
                if let Some(package) = entry.package {
                    affected.packages.insert(package);
                }
                affected.files.insert(relative_path.to_string_lossy().to_string());
            }
        }
    }

    affected
}

fn add_reverse_reachable_scope(
    build: &GraphBuildResult,
    file_id: oxgraph_graph::FileId,
    profile: EntrypointProfile,
    affected: &mut AffectedScope,
) {
    let reverse_nodes = build.module_graph.reverse_reachable_nodes_from_file(file_id);
    let reachable_nodes = build.module_graph.reachable_nodes(profile);

    for index in reverse_nodes.intersection(&reachable_nodes).copied() {
        match &build.module_graph.graph[index] {
            ModuleNode::File {
                relative_path,
                package,
                ..
            } => {
                affected.files.insert(relative_path.clone());
                if let Some(package) = package {
                    affected.packages.insert(package.clone());
                }
            }
            ModuleNode::Entrypoint { path, .. } => {
                let path = Path::new(path);
                affected.entrypoints.insert(
                    path.strip_prefix(&build.discovery.project_root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string(),
                );
            }
            ModuleNode::Package { name, .. } => {
                affected.packages.insert(name.clone());
            }
            ModuleNode::Workspace { .. } | ModuleNode::ExternalDependency { .. } => {}
        }
    }
}

fn finding_in_affected_scope(finding: &Finding, affected: &AffectedScope) -> bool {
    let subject_tokens = subject_tokens(&finding.subject);
    if !subject_tokens.is_empty() {
        return subject_tokens
            .iter()
            .any(|token| affected.files.contains(token));
    }

    if finding.evidence.iter().any(|evidence| {
        evidence
            .file
            .as_ref()
            .map(|file| normalize_subject_token(file))
            .is_some_and(|file| affected.files.contains(&file))
    }) {
        return true;
    }

    finding.package.as_ref().is_some_and(|package| affected.packages.contains(package))
}

fn subject_tokens(subject: &str) -> Vec<String> {
    if subject.contains(" -> ") {
        return subject
            .split(" -> ")
            .map(normalize_subject_token)
            .filter(|token| !token.is_empty())
            .collect();
    }

    let token = normalize_subject_token(subject);
    if token.is_empty() { Vec::new() } else { vec![token] }
}

fn normalize_subject_token(subject: &str) -> String {
    subject
        .split('#')
        .next()
        .unwrap_or(subject)
        .trim()
        .trim_start_matches("./")
        .to_string()
}

fn recover_deleted_paths(cache: Option<&AnalysisCache>, changed_scope: &mut ChangedScope) {
    changed_scope.recoverable_deleted.clear();
    changed_scope.unrecoverable_deleted.clear();

    for deleted in &changed_scope.deleted {
        let Some(cache) = cache else {
            changed_scope.unrecoverable_deleted.push(deleted.clone());
            continue;
        };
        match cache.lookup_path_index(deleted) {
            Ok(Some(_)) => changed_scope.recoverable_deleted.push(deleted.clone()),
            Ok(None) | Err(_) => changed_scope.unrecoverable_deleted.push(deleted.clone()),
        }
    }
}

fn refresh_summary(report: &mut AnalysisReport) {
    let (errors, warnings, infos) = finding_summary(&report.findings);
    report.summary.total_findings = report.findings.len();
    report.summary.errors = errors;
    report.summary.warnings = warnings;
    report.summary.infos = infos;
    if report.stats.baseline_applied && report.stats.new_findings == 0 {
        report.stats.new_findings = report.findings.len();
    }
}

fn build_proof_tree(nodes: Vec<String>) -> ProofNode {
    if nodes.is_empty() {
        return ProofNode {
            node: "unreachable".to_string(),
            relationship: "root".to_string(),
            children: Vec::new(),
        };
    }

    let mut iter = nodes.into_iter().rev();
    let mut current = ProofNode {
        node: iter.next().unwrap_or_default(),
        relationship: "target".to_string(),
        children: Vec::new(),
    };

    for node in iter {
        current = ProofNode {
            node,
            relationship: "depends-on".to_string(),
            children: vec![current],
        };
    }

    current
}

fn finding_summary(findings: &[Finding]) -> (usize, usize, usize) {
    findings.iter().fold((0, 0, 0), |(errors, warnings, infos), finding| {
        match finding.severity {
            oxgraph_report::FindingSeverity::Error => (errors + 1, warnings, infos),
            oxgraph_report::FindingSeverity::Warn => (errors, warnings + 1, infos),
            oxgraph_report::FindingSeverity::Info => (errors, warnings, infos + 1),
        }
    })
}

fn node_label(build: &GraphBuildResult, index: petgraph::graph::NodeIndex) -> String {
    match &build.module_graph.graph[index] {
        ModuleNode::Entrypoint { path, .. } => {
            let path = Path::new(path);
            path.strip_prefix(&build.discovery.project_root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string()
        }
        ModuleNode::File { relative_path, .. } => relative_path.clone(),
        ModuleNode::Package { name, .. }
        | ModuleNode::Workspace { name, .. }
        | ModuleNode::ExternalDependency { name } => name.clone(),
    }
}

pub fn render_module_graph_dot(build: &GraphBuildResult, findings: &[Finding]) -> String {
    let highlighted = findings
        .iter()
        .flat_map(|finding| subject_tokens(&finding.subject))
        .collect::<BTreeSet<_>>();

    let mut lines = vec![
        "digraph oxgraph {".to_string(),
        "  rankdir=LR;".to_string(),
        "  node [fontname=\"Menlo\"];".to_string(),
    ];

    for index in build.module_graph.graph.node_indices() {
        let node_id = format!("n{}", index.index());
        let label = node_label(build, index).replace('"', "\\\"");
        let (shape, fillcolor) = match &build.module_graph.graph[index] {
            ModuleNode::Workspace { .. } => ("box", "lightgray"),
            ModuleNode::Package { .. } => ("component", "lightblue"),
            ModuleNode::File { relative_path, .. } => {
                if highlighted.contains(relative_path) {
                    ("note", "mistyrose")
                } else {
                    ("note", "white")
                }
            }
            ModuleNode::Entrypoint { .. } => ("oval", "palegreen"),
            ModuleNode::ExternalDependency { .. } => ("hexagon", "khaki"),
        };
        lines.push(format!(
            "  {node_id} [label=\"{label}\", shape={shape}, style=filled, fillcolor={fillcolor}];"
        ));
    }

    for edge in build.module_graph.graph.edge_indices() {
        let Some((from, to)) = build.module_graph.graph.edge_endpoints(edge) else {
            continue;
        };
        lines.push(format!(
            "  n{} -> n{} [label=\"{}\"];",
            from.index(),
            to.index(),
            edge_label(build.module_graph.graph[edge])
        ));
    }

    lines.push("}".to_string());
    lines.join("\n")
}

const fn edge_label(edge: ModuleEdge) -> &'static str {
    match edge {
        ModuleEdge::StaticImportValue => "import",
        ModuleEdge::StaticImportType => "import-type",
        ModuleEdge::DynamicImport => "dynamic-import",
        ModuleEdge::Require => "require",
        ModuleEdge::SideEffectImport => "side-effect",
        ModuleEdge::ReExportNamed => "re-export",
        ModuleEdge::ReExportAll => "re-export-all",
        ModuleEdge::EntrypointToFile => "entrypoint",
        ModuleEdge::PackageToEntrypoint => "package-entrypoint",
        ModuleEdge::FileToDependency => "dependency",
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

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsScanOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub paths: Option<Vec<String>>,
    pub profile: Option<String>,
    pub changed_since: Option<String>,
    pub no_cache: Option<bool>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsImpactOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub target: String,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsExplainOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub query: String,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsDebugResolveOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub specifier: String,
    pub from: String,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsDebugEntrypointsOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsMigrateKnipOptions {
    pub cwd: Option<String>,
    pub file: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsMigrateDepcruiseOptions {
    pub cwd: Option<String>,
    pub file: Option<String>,
    pub node: Option<bool>,
}

#[cfg(feature = "napi")]
fn load_config_for_js(
    cwd: &Path,
    config_path: Option<&str>,
) -> napi::Result<OxgraphConfig> {
    match OxgraphConfig::load(cwd, config_path.map(Path::new)) {
        Ok(config) => Ok(config),
        Err(oxgraph_config::ConfigError::NotFound) => Ok(OxgraphConfig::default()),
        Err(err) => Err(napi::Error::from_reason(err.to_string())),
    }
}

#[cfg(feature = "napi")]
fn parse_profile(profile: Option<&str>) -> EntrypointProfile {
    match profile {
        Some("production") => EntrypointProfile::Production,
        Some("development") => EntrypointProfile::Development,
        _ => EntrypointProfile::Both,
    }
}

#[cfg(feature = "napi")]
#[napi]
pub fn scan_json(options: JsScanOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let report = scan_with_options(
        &cwd,
        &config,
        &options
            .paths
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
        parse_profile(options.profile.as_deref()),
        &ScanOptions {
            config_dir: resolve_config_dir(&cwd, options.config.as_deref()),
            changed_since: options.changed_since,
            no_cache: options.no_cache.unwrap_or(false),
        },
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report.report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn scan_dot_text(options: JsScanOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let execution = scan_with_options(
        &cwd,
        &config,
        &options
            .paths
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
        parse_profile(options.profile.as_deref()),
        &ScanOptions {
            config_dir: resolve_config_dir(&cwd, options.config.as_deref()),
            changed_since: options.changed_since,
            no_cache: options.no_cache.unwrap_or(false),
        },
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    Ok(render_module_graph_dot(&execution.build, &execution.report.findings))
}

#[cfg(feature = "napi")]
#[napi]
pub fn impact_json(options: JsImpactOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let report = impact(
        &cwd,
        &config,
        &options.target,
        parse_profile(options.profile.as_deref()),
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn explain_json(options: JsExplainOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let report = explain(
        &cwd,
        &config,
        &options.query,
        parse_profile(options.profile.as_deref()),
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn load_config_json(cwd: Option<String>, config: Option<String>) -> napi::Result<String> {
    let cwd = cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, config.as_deref())?;
    serde_json::to_string(&config).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn debug_resolve_text(options: JsDebugResolveOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    Ok(debug_resolve(&cwd, &config, &options.specifier, Path::new(&options.from)))
}

#[cfg(feature = "napi")]
#[napi]
pub fn debug_entrypoints_json(options: JsDebugEntrypointsOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let entrypoints = debug_entrypoints(&cwd, &config, parse_profile(options.profile.as_deref()))
        .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&entrypoints).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn migrate_knip_json(options: JsMigrateKnipOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let output = crate::migrate::migrate_knip(&cwd, options.file.as_deref().map(Path::new))
        .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&output).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn migrate_depcruise_json(options: JsMigrateDepcruiseOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let output = crate::migrate::migrate_depcruise(
        &cwd,
        options.file.as_deref().map(Path::new),
        options.node.unwrap_or(false),
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&output).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
fn resolve_config_dir(cwd: &Path, config: Option<&str>) -> Option<PathBuf> {
    config.map(|config| {
        let path = Path::new(config);
        if path.is_absolute() {
            path.parent().map_or_else(|| cwd.to_path_buf(), Path::to_path_buf)
        } else {
            cwd.join(path)
                .parent()
                .map_or_else(|| cwd.to_path_buf(), Path::to_path_buf)
        }
    })
}
