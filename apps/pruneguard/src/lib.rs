use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use miette::IntoDiagnostic;
use pruneguard_cache::AnalysisCache;
use pruneguard_config::PruneguardConfig;
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_git::{ChangedScope, collect_changed_scope};
use pruneguard_graph::{
    BuildOptions, GraphBuildResult, ModuleEdge, ModuleNode, build_graph, build_graph_with_options,
};
use pruneguard_report::{
    AnalysisReport, ConfidenceCounts, ExplainQueryKind, ExplainReport, Finding, FindingConfidence,
    ImpactReport, ProofNode, Summary,
};

#[cfg(feature = "napi")]
use napi_derive::napi;

#[cfg(feature = "napi")]
mod migrate;

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub config_dir: Option<PathBuf>,
    pub changed_since: Option<String>,
    pub focus: Option<String>,
    pub no_cache: bool,
    pub no_baseline: bool,
    pub require_full_scope: bool,
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

#[derive(Debug, Clone, Default)]
pub struct ImpactOptions {
    pub focus: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExplainOptions {
    pub focus: Option<String>,
}

#[derive(Debug, Clone)]
struct FocusFilter {
    matcher: GlobSet,
}

/// Run a full scan and return the analysis report.
pub fn scan(
    cwd: &Path,
    config: &PruneguardConfig,
    paths: &[PathBuf],
    profile: EntrypointProfile,
) -> miette::Result<AnalysisReport> {
    Ok(scan_with_options(cwd, config, paths, profile, &ScanOptions::default())?.report)
}

pub fn scan_with_options(
    cwd: &Path,
    config: &PruneguardConfig,
    paths: &[PathBuf],
    profile: EntrypointProfile,
    options: &ScanOptions,
) -> miette::Result<ScanExecution> {
    let scan_roots = normalize_scan_roots(cwd, paths);
    let discovery_cwd = scan_roots.first().map_or_else(|| cwd.to_path_buf(), Clone::clone);
    let discovery = pruneguard_discovery::discover(&discovery_cwd, config)?;
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
    if options.require_full_scope
        && build.stats.partial_scope
        && dead_code_analyzers_enabled(config)
    {
        miette::bail!(
            "{}",
            build
                .stats
                .partial_scope_reason
                .as_deref()
                .unwrap_or("partial-scope scan detected while --require-full-scope was enabled")
        );
    }
    let findings = pruneguard_analyzers::run_analyzers(&build, config, profile);
    let mut report = report_from_build(cwd, &build, findings, profile);
    report.stats.full_scope_required = options.require_full_scope;

    if let Some(reference) = &options.changed_since {
        let mut changed_scope =
            collect_changed_scope(&build.discovery.project_root, reference, &scan_roots)?;
        recover_deleted_paths(cache.as_ref(), &mut changed_scope);
        apply_changed_scope(&mut report, &build, &changed_scope, cache.as_ref(), profile);
    }

    if !options.no_baseline
        && let Some(baseline) =
            load_baseline(options.config_dir.as_deref(), &build.discovery.project_root)?
    {
        apply_baseline(&mut report, &baseline);
    }

    if let Some(focus) = compile_focus_filter(options.focus.as_deref())? {
        apply_focus_to_scan_report(&mut report, &build, &focus);
    }

    refresh_summary(&mut report);

    Ok(ScanExecution { report, build })
}

/// Compute the blast radius for a target.
pub fn impact(
    cwd: &Path,
    config: &PruneguardConfig,
    target: &str,
    profile: EntrypointProfile,
) -> miette::Result<ImpactReport> {
    impact_with_options(cwd, config, target, profile, &ImpactOptions::default())
}

pub fn impact_with_options(
    cwd: &Path,
    config: &PruneguardConfig,
    target: &str,
    profile: EntrypointProfile,
    options: &ImpactOptions,
) -> miette::Result<ImpactReport> {
    let build = build_graph(cwd, config, &[], profile)?;
    let target_file = build
        .find_file(target)
        .ok_or_else(|| miette::miette!("target `{target}` did not match a tracked file"))?;
    let file_id = build
        .module_graph
        .file_id(&target_file.file.path.to_string_lossy())
        .ok_or_else(|| miette::miette!("target `{target}` is not present in the module graph"))?;

    let analysis = pruneguard_analyzers::impact::analyze(&build, file_id, profile);
    let proofs = build
        .module_graph
        .shortest_path_to_file(file_id, profile)
        .map(|path| path.iter().map(|index| node_label(&build, *index)).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut report = ImpactReport {
        target: target_file.file.relative_path.to_string_lossy().to_string(),
        affected_entrypoints: analysis.affected_entrypoints,
        affected_packages: analysis.affected_packages,
        affected_files: analysis.affected_files,
        evidence: if proofs.is_empty() {
            Vec::new()
        } else {
            vec![pruneguard_report::Evidence {
                kind: "path".to_string(),
                file: Some(target_file.file.relative_path.to_string_lossy().to_string()),
                line: None,
                description: format!("One entrypoint path to the target: {}", proofs.join(" -> ")),
            }]
        },
        focus_filtered: false,
    };

    if let Some(focus) = compile_focus_filter(options.focus.as_deref())? {
        apply_focus_to_impact_report(&mut report, &build, &focus);
    }

    Ok(report)
}

/// Explain a finding or path.
pub fn explain(
    cwd: &Path,
    config: &PruneguardConfig,
    query: &str,
    profile: EntrypointProfile,
) -> miette::Result<ExplainReport> {
    explain_with_options(cwd, config, query, profile, &ExplainOptions::default())
}

pub fn explain_with_options(
    cwd: &Path,
    config: &PruneguardConfig,
    query: &str,
    profile: EntrypointProfile,
    options: &ExplainOptions,
) -> miette::Result<ExplainReport> {
    let build = build_graph(cwd, config, &[], profile)?;
    let findings = pruneguard_analyzers::run_analyzers(&build, config, profile);
    let matched_finding_id = findings.iter().any(|finding| finding.id == query);
    let related_findings = findings
        .iter()
        .filter(|finding| finding.id == query || finding.subject == query)
        .cloned()
        .collect::<Vec<_>>();

    let target_query = related_findings.first().map_or(query, |finding| finding.subject.as_str());
    let matched_file = build
        .find_file(target_query)
        .or_else(|| target_query.split_once('#').and_then(|(path, _)| build.find_file(path)));

    let matched_node = matched_file
        .map(|file| file.file.relative_path.to_string_lossy().to_string())
        .or_else(|| related_findings.first().map(|finding| finding.subject.clone()));
    let query_kind = if matched_finding_id {
        ExplainQueryKind::Finding
    } else if target_query.contains('#') {
        ExplainQueryKind::Export
    } else {
        ExplainQueryKind::File
    };

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

    let mut report = ExplainReport {
        query: query.to_string(),
        matched_node,
        query_kind,
        proofs,
        related_findings,
        focus_filtered: false,
    };

    if let Some(focus) = compile_focus_filter(options.focus.as_deref())? {
        apply_focus_to_explain_report(&mut report, &build, &focus);
    }

    Ok(report)
}

/// Debug: list all detected entrypoints.
pub fn debug_entrypoints(
    cwd: &Path,
    config: &PruneguardConfig,
    profile: EntrypointProfile,
) -> miette::Result<Vec<String>> {
    let build = build_graph(cwd, config, &[], profile)?;
    Ok(build.entrypoint_seeds.iter().map(ToString::to_string).collect())
}

/// Debug: resolve a specifier from a file.
pub fn debug_resolve(cwd: &Path, config: &PruneguardConfig, specifier: &str, from: &Path) -> String {
    pruneguard_resolver::debug_resolve(cwd, &config.resolver, specifier, from)
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
            first.evidence.push(pruneguard_report::Evidence {
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

    report.findings.retain(|finding| finding_in_affected_scope(finding, &affected));
}

fn apply_focus_to_scan_report(
    report: &mut AnalysisReport,
    build: &GraphBuildResult,
    focus: &FocusFilter,
) {
    report.stats.focus_applied = true;
    report.stats.focused_files = report
        .inventories
        .files
        .iter()
        .filter(|file| focus_matches_path(focus, &file.path))
        .count();
    report.findings.retain(|finding| finding_matches_focus(focus, finding, build));
    report.stats.focused_findings = report.findings.len();
}

fn apply_focus_to_impact_report(
    report: &mut ImpactReport,
    build: &GraphBuildResult,
    focus: &FocusFilter,
) {
    let original_files = report.affected_files.len();
    let original_packages = report.affected_packages.len();
    let original_entrypoints = report.affected_entrypoints.len();
    let original_evidence = report.evidence.len();
    report.affected_files.retain(|path| focus_matches_path(focus, path));

    let focused_packages = report
        .affected_files
        .iter()
        .filter_map(|path| build.find_file(path))
        .filter_map(|file| file.file.package.clone())
        .collect::<BTreeSet<_>>();
    report.affected_packages.retain(|package| focused_packages.contains(package));

    report.affected_entrypoints.retain(|path| {
        let relative = normalize_focus_token(
            Path::new(path)
                .strip_prefix(&build.discovery.project_root)
                .unwrap_or_else(|_| Path::new(path))
                .to_string_lossy()
                .as_ref(),
        );
        focus_matches_path(focus, &relative)
    });

    report.evidence.retain(|evidence| {
        evidence
            .file
            .as_ref()
            .is_some_and(|file| focus_matches_path(focus, &normalize_focus_token(file)))
            || evidence
                .description
                .split(" -> ")
                .any(|segment| focus_matches_path(focus, &normalize_focus_token(segment)))
    });

    report.focus_filtered = report.affected_files.len() != original_files
        || report.affected_packages.len() != original_packages
        || report.affected_entrypoints.len() != original_entrypoints
        || report.evidence.len() != original_evidence;
}

fn apply_focus_to_explain_report(
    report: &mut ExplainReport,
    build: &GraphBuildResult,
    focus: &FocusFilter,
) {
    let original_findings = report.related_findings.len();
    let original_proofs = report.proofs.len();
    let matched_outside_focus = report
        .matched_node
        .as_deref()
        .is_some_and(|node| !focus_matches_path(focus, &normalize_subject_token(node)));
    report.related_findings.retain(|finding| finding_matches_focus(focus, finding, build));
    report.proofs =
        report.proofs.iter().filter_map(|proof| filter_proof_node(proof, focus)).collect();
    report.focus_filtered = report.related_findings.len() != original_findings
        || report.proofs.len() != original_proofs
        || matched_outside_focus;
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
        affected.files.insert(file.file.relative_path.to_string_lossy().to_string());
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
    file_id: pruneguard_graph::FileId,
    profile: EntrypointProfile,
    affected: &mut AffectedScope,
) {
    let reverse_nodes = build.module_graph.reverse_reachable_nodes_from_file(file_id);
    let reachable_nodes = build.module_graph.reachable_nodes(profile);

    for index in reverse_nodes.intersection(&reachable_nodes).copied() {
        match &build.module_graph.graph[index] {
            ModuleNode::File { relative_path, package, .. } => {
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
        return subject_tokens.iter().any(|token| affected.files.contains(token));
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

fn finding_matches_focus(focus: &FocusFilter, finding: &Finding, build: &GraphBuildResult) -> bool {
    let subject_tokens = subject_tokens(&finding.subject);
    if subject_tokens.iter().any(|token| focus_matches_path(focus, token)) {
        return true;
    }

    if finding.evidence.iter().any(|evidence| {
        evidence
            .file
            .as_ref()
            .map(|file| normalize_focus_token(file))
            .is_some_and(|file| focus_matches_path(focus, &file))
    }) {
        return true;
    }

    if matches!(finding.category, pruneguard_report::FindingCategory::UnusedPackage)
        && let Some(package) = &finding.package
    {
        return build.files.iter().any(|file| {
            file.file.package.as_ref() == Some(package)
                && focus_matches_path(focus, &file.file.relative_path.to_string_lossy())
        });
    }

    false
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
    normalize_focus_token(
        subject.split('#').next().unwrap_or(subject).trim().trim_start_matches("./"),
    )
}

fn normalize_focus_token(subject: &str) -> String {
    subject.trim().trim_start_matches("./").trim_start_matches('/').to_string()
}

fn compile_focus_filter(pattern: Option<&str>) -> miette::Result<Option<FocusFilter>> {
    let Some(pattern) = pattern.filter(|pattern| !pattern.trim().is_empty()) else {
        return Ok(None);
    };

    let mut builder = GlobSetBuilder::new();
    builder.add(Glob::new(pattern).map_err(|err| miette::miette!("{err}"))?);
    let matcher = builder.build().map_err(|err| miette::miette!("{err}"))?;
    Ok(Some(FocusFilter { matcher }))
}

fn focus_matches_path(focus: &FocusFilter, path: &str) -> bool {
    focus.matcher.is_match(normalize_focus_token(path))
}

fn filter_proof_node(node: &ProofNode, focus: &FocusFilter) -> Option<ProofNode> {
    let children = node
        .children
        .iter()
        .filter_map(|child| filter_proof_node(child, focus))
        .collect::<Vec<_>>();
    if focus_matches_path(focus, &node.node) || !children.is_empty() {
        return Some(ProofNode {
            node: node.node.clone(),
            relationship: node.relationship.clone(),
            children,
        });
    }
    None
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
    report.stats.confidence_counts = confidence_counts(&report.findings);
    if report.stats.baseline_applied && report.stats.new_findings == 0 {
        report.stats.new_findings = report.findings.len();
    }
}

fn confidence_counts(findings: &[Finding]) -> ConfidenceCounts {
    findings.iter().fold(ConfidenceCounts::default(), |mut counts, finding| {
        match finding.confidence {
            FindingConfidence::High => counts.high += 1,
            FindingConfidence::Medium => counts.medium += 1,
            FindingConfidence::Low => counts.low += 1,
        }
        counts
    })
}

const fn dead_code_analyzers_enabled(config: &PruneguardConfig) -> bool {
    !matches!(config.analysis.unused_exports, pruneguard_config::AnalysisSeverity::Off)
        || !matches!(config.analysis.unused_files, pruneguard_config::AnalysisSeverity::Off)
        || !matches!(config.analysis.unused_packages, pruneguard_config::AnalysisSeverity::Off)
        || !matches!(config.analysis.unused_dependencies, pruneguard_config::AnalysisSeverity::Off)
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
        current =
            ProofNode { node, relationship: "depends-on".to_string(), children: vec![current] };
    }

    current
}

fn finding_summary(findings: &[Finding]) -> (usize, usize, usize) {
    findings.iter().fold((0, 0, 0), |(errors, warnings, infos), finding| match finding.severity {
        pruneguard_report::FindingSeverity::Error => (errors + 1, warnings, infos),
        pruneguard_report::FindingSeverity::Warn => (errors, warnings + 1, infos),
        pruneguard_report::FindingSeverity::Info => (errors, warnings, infos + 1),
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
        "digraph pruneguard {".to_string(),
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
        .map(|path| if path.is_absolute() { path.clone() } else { cwd.join(path) })
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
    pub focus: Option<String>,
    pub no_cache: Option<bool>,
    pub no_baseline: Option<bool>,
    pub require_full_scope: Option<bool>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsImpactOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub target: String,
    pub profile: Option<String>,
    pub focus: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsExplainOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub query: String,
    pub profile: Option<String>,
    pub focus: Option<String>,
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
fn load_config_for_js(cwd: &Path, config_path: Option<&str>) -> napi::Result<PruneguardConfig> {
    match PruneguardConfig::load(cwd, config_path.map(Path::new)) {
        Ok(config) => Ok(config),
        Err(pruneguard_config::ConfigError::NotFound) => Ok(PruneguardConfig::default()),
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
        &options.paths.unwrap_or_default().into_iter().map(PathBuf::from).collect::<Vec<_>>(),
        parse_profile(options.profile.as_deref()),
        &ScanOptions {
            config_dir: resolve_config_dir(&cwd, options.config.as_deref()),
            changed_since: options.changed_since,
            focus: options.focus,
            no_cache: options.no_cache.unwrap_or(false),
            no_baseline: options.no_baseline.unwrap_or(false),
            require_full_scope: options.require_full_scope.unwrap_or(false),
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
        &options.paths.unwrap_or_default().into_iter().map(PathBuf::from).collect::<Vec<_>>(),
        parse_profile(options.profile.as_deref()),
        &ScanOptions {
            config_dir: resolve_config_dir(&cwd, options.config.as_deref()),
            changed_since: options.changed_since,
            focus: options.focus,
            no_cache: options.no_cache.unwrap_or(false),
            no_baseline: options.no_baseline.unwrap_or(false),
            require_full_scope: options.require_full_scope.unwrap_or(false),
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
    let report = impact_with_options(
        &cwd,
        &config,
        &options.target,
        parse_profile(options.profile.as_deref()),
        &ImpactOptions { focus: options.focus },
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
    let report = explain_with_options(
        &cwd,
        &config,
        &options.query,
        parse_profile(options.profile.as_deref()),
        &ExplainOptions { focus: options.focus },
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
            cwd.join(path).parent().map_or_else(|| cwd.to_path_buf(), Path::to_path_buf)
        }
    })
}
