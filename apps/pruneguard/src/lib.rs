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
    AnalysisReport, ConfidenceCounts, DeletionOrderEntry, ExplainQueryKind, ExplainReport, Finding,
    FindingConfidence, FindingSeverity, FixPlanPhase, FixPlanReport, ImpactReport, ProofNode,
    RecommendedAction, RecommendedActionKind, RemediationAction, RemediationActionKind,
    RemediationStep, ReviewReport, ReviewTrust, RiskLevel, SafeDeleteCandidate,
    SafeDeleteClassification, SafeDeleteReport, SuggestRulesReport, Summary,
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

    // --- Compute compatibility report and enrich findings with trust context ---
    let compat = compute_compat_report_from_build(&build);
    report.stats.frameworks_detected.clone_from(&compat.supported_frameworks);
    report.stats.heuristic_frameworks.clone_from(&compat.heuristic_frameworks);
    report.stats.heuristic_entrypoints =
        build.entrypoints.iter().filter(|ep| ep.heuristic.unwrap_or(false)).count();
    report.stats.compatibility_warnings =
        compat.warnings.iter().map(|w| w.message.clone()).collect();

    // Annotate findings with trust notes and framework context.
    for finding in &mut report.findings {
        let notes = compat.trust_notes_for_path(&finding.subject);
        if !notes.is_empty() {
            finding.trust_notes = Some(notes);
        }

        let fw_context: Vec<String> =
            build
                .entrypoints
                .iter()
                .filter_map(|ep| {
                    ep.framework.as_ref().filter(|_| {
                        // Attach framework context when the finding subject is in the
                        // same workspace as a framework-contributed entrypoint.
                        finding.workspace.as_ref() == ep.workspace.as_ref()
                            || finding.subject.split('#').next().is_some_and(|path| {
                                ep.path.contains(path) || path.contains(&ep.path)
                            })
                    })
                })
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        if !fw_context.is_empty() {
            finding.framework_context = Some(fw_context);
        }
    }

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

#[derive(Debug, Clone, Default)]
pub struct ReviewOptions {
    pub config_dir: Option<PathBuf>,
    pub base_ref: Option<String>,
    pub no_cache: bool,
    pub no_baseline: bool,
    pub strict_trust: bool,
}

#[derive(Debug, Clone)]
pub struct SafeDeleteOptions {
    pub config_dir: Option<PathBuf>,
    pub no_cache: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FixPlanOptions {
    pub config_dir: Option<PathBuf>,
    pub no_cache: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SuggestRulesOptions {
    pub config_dir: Option<PathBuf>,
    pub no_cache: bool,
}

// --- Framework debug and compatibility report types ---

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkDebugReport {
    pub detected_packs: Vec<DetectedPackInfo>,
    pub all_entrypoints: Vec<FrameworkEntrypointInfo>,
    pub all_ignore_patterns: Vec<String>,
    pub all_classification_rules: Vec<ClassificationRuleInfo>,
    pub heuristic_detections: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedPackInfo {
    pub name: String,
    pub confidence: String,
    pub signals: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkEntrypointInfo {
    pub path: String,
    pub framework: String,
    pub kind: String,
    pub reason: String,
    pub heuristic: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationRuleInfo {
    pub pattern: String,
    pub classification: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReportOutput {
    pub supported_frameworks: Vec<String>,
    pub heuristic_frameworks: Vec<String>,
    pub unsupported_signals: Vec<UnsupportedSignalInfo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedSignalInfo {
    pub signal: String,
    pub source: String,
    pub suggestion: Option<String>,
}

/// Debug framework detection: lists detected packs, contributed entrypoints,
/// ignore patterns, classification rules, and heuristic detections.
pub fn debug_frameworks(
    cwd: &Path,
    profile: EntrypointProfile,
) -> miette::Result<FrameworkDebugReport> {
    let config = match PruneguardConfig::load(cwd, None) {
        Ok(config) => config,
        Err(pruneguard_config::ConfigError::NotFound) => PruneguardConfig::default(),
        Err(err) => return Err(err.into()),
    };
    let build = build_graph(cwd, &config, &[], profile)?;

    let detected_packs: Vec<DetectedPackInfo> = build
        .stats
        .frameworks_detected
        .iter()
        .map(|name| DetectedPackInfo {
            name: name.clone(),
            confidence: "exact".to_string(),
            signals: vec![format!("detected in project for profile {}", profile.as_str())],
            reasons: vec!["matched framework pack definition".to_string()],
        })
        .chain(build.stats.heuristic_frameworks.iter().map(|name| DetectedPackInfo {
            name: name.clone(),
            confidence: "heuristic".to_string(),
            signals: vec![format!("heuristic detection for profile {}", profile.as_str())],
            reasons: vec!["matched via file-pattern heuristics".to_string()],
        }))
        .collect();

    let all_entrypoints: Vec<FrameworkEntrypointInfo> = build
        .entrypoints
        .iter()
        .filter_map(|ep| {
            // Only include entrypoints that came from framework detection.
            if ep.source.contains("framework") || ep.source.contains("heuristic") {
                Some(FrameworkEntrypointInfo {
                    path: ep.path.clone(),
                    framework: ep.framework.clone().unwrap_or_else(|| ep.source.clone()),
                    kind: ep.kind.clone(),
                    reason: ep.reason.clone().unwrap_or_default(),
                    heuristic: ep.heuristic.unwrap_or(false),
                })
            } else {
                None
            }
        })
        .collect();

    let all_ignore_patterns = config.ignore_patterns;

    // Classification rules are not a top-level config field; return empty for now.
    let all_classification_rules: Vec<ClassificationRuleInfo> = Vec::new();

    let heuristic_detections: Vec<String> =
        build.stats.heuristic_frameworks.iter().map(|name| format!("heuristic: {name}")).collect();

    Ok(FrameworkDebugReport {
        detected_packs,
        all_entrypoints,
        all_ignore_patterns,
        all_classification_rules,
        heuristic_detections,
    })
}

/// Generate a compatibility report: lists supported and heuristic frameworks,
/// unsupported signals, and warnings.
pub fn compatibility_report(
    cwd: &Path,
    profile: EntrypointProfile,
) -> miette::Result<CompatibilityReportOutput> {
    let config = match PruneguardConfig::load(cwd, None) {
        Ok(config) => config,
        Err(pruneguard_config::ConfigError::NotFound) => PruneguardConfig::default(),
        Err(err) => return Err(err.into()),
    };
    let build = build_graph(cwd, &config, &[], profile)?;
    Ok(build_compat_output_from_stats(&build.stats))
}

/// Convenience wrapper used during review strict-trust checks.
fn compatibility_report_from_scan(
    _cwd: &Path,
    _profile: EntrypointProfile,
    execution: &ScanExecution,
) -> CompatibilityReportOutput {
    build_compat_output_from_stats(&execution.build.stats)
}

/// Build a `CompatibilityReportOutput` from graph build stats.
fn build_compat_output_from_stats(stats: &pruneguard_report::Stats) -> CompatibilityReportOutput {
    let supported_frameworks = stats.frameworks_detected.clone();
    let heuristic_frameworks = stats.heuristic_frameworks.clone();

    let mut unsupported_signals = Vec::new();
    let mut warnings = Vec::new();

    for warning in &stats.compatibility_warnings {
        warnings.push(warning.clone());
    }

    for fw in &heuristic_frameworks {
        unsupported_signals.push(UnsupportedSignalInfo {
            signal: fw.clone(),
            source: "heuristic-detection".to_string(),
            suggestion: Some(format!(
                "Consider adding a framework pack for `{fw}` to improve accuracy."
            )),
        });
        warnings.push(format!(
            "Framework `{fw}` was detected heuristically. Entrypoint coverage may be incomplete."
        ));
    }

    if stats.files_resolved > 0 {
        #[allow(clippy::cast_precision_loss)]
        let pressure = stats.unresolved_specifiers as f64
            / (stats.files_resolved + stats.unresolved_specifiers) as f64;
        if pressure > 0.05 {
            warnings.push(format!(
                "Unresolved pressure is {:.1}%. Findings may be less accurate.",
                pressure * 100.0
            ));
        }
    }

    CompatibilityReportOutput {
        supported_frameworks,
        heuristic_frameworks,
        unsupported_signals,
        warnings,
    }
}

/// Compute a full `pruneguard_compat::CompatibilityReport` from the graph build
/// result by running framework detection and collecting trust notes from all
/// detected packs against each workspace.
fn compute_compat_report_from_build(
    build: &GraphBuildResult,
) -> pruneguard_compat::CompatibilityReport {
    let packs = pruneguard_frameworks::built_in_packs();
    let mut all_detections = Vec::new();
    let mut all_trust_notes = Vec::new();

    // Use the first (root) workspace manifest for the compat report.
    let root_manifest = build.discovery.workspaces.values().next().map(|w| &w.manifest);

    for workspace in build.discovery.workspaces.values() {
        let detections =
            pruneguard_frameworks::detect_all_frameworks(&workspace.root, &workspace.manifest);
        all_detections.extend(detections);

        for pack in &packs {
            if pack.detect(&workspace.root, &workspace.manifest) {
                let notes = pack.trust_notes(&workspace.root, &workspace.manifest);
                all_trust_notes.extend(notes);
            }
        }
    }

    // Deduplicate framework detections by name (keep first, which is the
    // highest confidence since exact wins over heuristic).
    let mut seen_names = BTreeSet::new();
    all_detections.retain(|d| seen_names.insert(d.name));

    let manifest = root_manifest.cloned().unwrap_or_default();
    pruneguard_compat::CompatibilityReport::compute(&all_detections, &all_trust_notes, &manifest)
}

/// Review a branch for CI/agent gating.
///
/// Combines full-scope scan, `--changed-since`, baseline, and trust summary
/// into a single report with blocking vs advisory findings.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
pub fn review(
    cwd: &Path,
    config: &PruneguardConfig,
    profile: EntrypointProfile,
    options: &ReviewOptions,
) -> miette::Result<ReviewReport> {
    let scan_options = ScanOptions {
        config_dir: options.config_dir.clone(),
        changed_since: options.base_ref.clone(),
        no_cache: options.no_cache,
        no_baseline: options.no_baseline,
        require_full_scope: false,
        ..ScanOptions::default()
    };
    let execution = scan_with_options(cwd, config, &[], profile, &scan_options)?;
    let report = &execution.report;

    let changed_files = if options.base_ref.is_some() {
        let scope = collect_changed_scope(
            &execution.build.discovery.project_root,
            options.base_ref.as_deref().unwrap_or("HEAD~1"),
            &[],
        )?;
        scope.changed_paths().iter().map(|path| path.to_string_lossy().to_string()).collect()
    } else {
        Vec::new()
    };

    let new_findings = report.findings.clone();
    let mut blocking_findings = new_findings
        .iter()
        .filter(|f| {
            f.confidence == FindingConfidence::High
                && matches!(f.severity, FindingSeverity::Error | FindingSeverity::Warn)
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut advisory_findings = new_findings
        .iter()
        .filter(|f| {
            f.confidence != FindingConfidence::High || matches!(f.severity, FindingSeverity::Info)
        })
        .cloned()
        .collect::<Vec<_>>();

    let unresolved_pressure = if report.stats.files_resolved > 0 {
        report.stats.unresolved_specifiers as f64
            / (report.stats.files_resolved + report.stats.unresolved_specifiers) as f64
    } else {
        0.0
    };

    let mut recommendations = Vec::new();
    let mut recommended_actions: Vec<RecommendedAction> = Vec::new();
    let mut priority = 1usize;

    if !blocking_findings.is_empty() {
        recommendations.push(format!(
            "{} blocking finding(s) should be resolved before merge.",
            blocking_findings.len()
        ));

        // Collect safe-deletable targets (unused files/exports with high confidence).
        let safe_delete_targets: Vec<String> = blocking_findings
            .iter()
            .filter(|f| {
                matches!(f.code.as_str(), "unused-file" | "unused-export")
                    && f.confidence == FindingConfidence::High
            })
            .map(|f| f.subject.clone())
            .collect();

        if !safe_delete_targets.is_empty() {
            recommended_actions.push(RecommendedAction {
                kind: RecommendedActionKind::RunSafeDelete,
                description: format!(
                    "Run safe-delete on {} unused target(s) to remove dead code.",
                    safe_delete_targets.len()
                ),
                priority,
                command: Some(format!("pruneguard safe-delete {}", safe_delete_targets.join(" "))),
                targets: safe_delete_targets,
            });
            priority += 1;
        }

        // Collect targets that need fix-plan (non-deletable blocking findings).
        let fix_plan_targets: Vec<String> = blocking_findings
            .iter()
            .filter(|f| {
                !matches!(f.code.as_str(), "unused-file" | "unused-export")
                    || f.confidence != FindingConfidence::High
            })
            .map(|f| f.subject.clone())
            .collect();

        if !fix_plan_targets.is_empty() {
            recommended_actions.push(RecommendedAction {
                kind: RecommendedActionKind::RunFixPlan,
                description: format!(
                    "Generate a fix plan for {} blocking finding(s).",
                    fix_plan_targets.len()
                ),
                priority,
                command: Some(format!("pruneguard fix-plan {}", fix_plan_targets.join(" "))),
                targets: fix_plan_targets,
            });
            priority += 1;
        }

        recommended_actions.push(RecommendedAction {
            kind: RecommendedActionKind::ResolveBlocking,
            description: format!(
                "Resolve all {} blocking finding(s) before merge.",
                blocking_findings.len()
            ),
            priority,
            command: None,
            targets: blocking_findings.iter().map(|f| f.id.clone()).collect(),
        });
        priority += 1;
    }

    if unresolved_pressure > 0.05 {
        recommendations.push(format!(
            "Unresolved pressure is {:.1}% — findings may be less accurate. Consider configuring resolver paths.",
            unresolved_pressure * 100.0
        ));
        recommended_actions.push(RecommendedAction {
            kind: RecommendedActionKind::FixResolverConfig,
            description: format!(
                "Unresolved pressure is {:.1}%. Configure resolver paths to improve accuracy.",
                unresolved_pressure * 100.0
            ),
            priority,
            command: None,
            targets: Vec::new(),
        });
        priority += 1;
    }

    if report.stats.partial_scope {
        recommendations.push(
            "Partial-scope analysis was used. Dead-code findings are advisory only.".to_string(),
        );
        recommended_actions.push(RecommendedAction {
            kind: RecommendedActionKind::RunFullScope,
            description: "Run a full-scope scan for higher-confidence dead-code detection."
                .to_string(),
            priority,
            command: Some("pruneguard scan".to_string()),
            targets: Vec::new(),
        });
        priority += 1;
    }

    if !advisory_findings.is_empty() {
        recommended_actions.push(RecommendedAction {
            kind: RecommendedActionKind::ReviewAdvisory,
            description: format!(
                "Review {} advisory finding(s) for potential improvements.",
                advisory_findings.len()
            ),
            priority,
            command: None,
            targets: advisory_findings.iter().map(|f| f.id.clone()).collect(),
        });
        priority += 1;
    }

    if blocking_findings.is_empty() && advisory_findings.is_empty() {
        recommendations.push("No new findings. Branch is clean.".to_string());
        recommended_actions.push(RecommendedAction {
            kind: RecommendedActionKind::None,
            description: "No new findings. Branch is clean.".to_string(),
            priority,
            command: None,
            targets: Vec::new(),
        });
        priority += 1;
    }

    let _ = priority; // suppress unused assignment warning

    let mut proposed_actions: Vec<RemediationAction> = blocking_findings
        .iter()
        .filter_map(|finding| {
            let kind = finding.primary_action_kind?;
            let steps = generate_remediation_steps(kind, &finding.subject);
            let preconditions = generate_preconditions(kind, &finding.subject);
            let verification = generate_verification_steps(kind, &finding.subject);
            let risk = risk_for_action(kind);
            Some(RemediationAction {
                id: format!("review-{}", finding.id),
                kind,
                targets: vec![finding.subject.clone()],
                why: finding.message.clone(),
                preconditions,
                steps,
                verification,
                risk,
                confidence: finding.confidence,
                rank: None,
                phase: Some(phase_name_for_action(kind).to_string()),
                finding_ids: vec![finding.id.clone()],
            })
        })
        .collect();

    // Apply ranking policy: sort by minimal blast radius first, then high
    // confidence first, then low unresolved pressure first.
    rank_remediation_actions(&mut proposed_actions);

    // --- strict-trust enforcement ---
    //
    // When `strict_trust` is enabled, only findings that meet all trust
    // criteria remain blocking:
    //   1. Full-scope analysis (not partial)
    //   2. High confidence
    //   3. Low unresolved pressure (<=5%)
    //   4. No heuristic-only framework detection
    //   5. No unsupported signals
    //
    // If trust is insufficient, ALL findings are moved to advisory.
    let mut strict_trust_applied = report.stats.strict_trust_applied;
    let mut compatibility_warnings = report.stats.compatibility_warnings.clone();

    if options.strict_trust {
        let compat = compatibility_report_from_scan(cwd, profile, &execution);
        let has_heuristic = !compat.heuristic_frameworks.is_empty();
        let has_unsupported = !compat.unsupported_signals.is_empty();
        let high_pressure = unresolved_pressure > 0.05;

        let insufficient_trust =
            has_heuristic || has_unsupported || high_pressure || report.stats.partial_scope;

        if insufficient_trust {
            strict_trust_applied = true;

            if report.stats.partial_scope {
                compatibility_warnings
                    .push("Partial-scope analysis: dead-code findings may be incomplete.".into());
            }
            if has_heuristic {
                compatibility_warnings.push(format!(
                    "Heuristic framework detection in use: {}. Entrypoints may be approximate.",
                    compat.heuristic_frameworks.join(", ")
                ));
            }
            if has_unsupported {
                compatibility_warnings.push(format!(
                    "{} unsupported signal(s) detected. Some framework conventions are not tracked.",
                    compat.unsupported_signals.len()
                ));
            }
            if high_pressure {
                compatibility_warnings.push(format!(
                    "Unresolved pressure is {:.1}%. Findings may be inaccurate.",
                    unresolved_pressure * 100.0
                ));
            }

            // Move all blocking findings to advisory.
            advisory_findings.append(&mut blocking_findings);
            recommendations.push(
                "strict-trust: trust conditions not met — all findings downgraded to advisory."
                    .to_string(),
            );
        }
    }

    Ok(ReviewReport {
        base_ref: options.base_ref.clone(),
        changed_files,
        new_findings,
        blocking_findings,
        advisory_findings,
        trust: ReviewTrust {
            full_scope: !report.stats.partial_scope,
            baseline_applied: report.stats.baseline_applied,
            unresolved_pressure,
            confidence_counts: report.stats.confidence_counts.clone(),
            execution_mode: report.stats.execution_mode,
        },
        recommendations,
        recommended_actions,
        proposed_actions,
        execution_mode: report.stats.execution_mode,
        latency_ms: None,
        compatibility_warnings,
        strict_trust_applied,
    })
}

/// Evaluate targets for safe deletion.
///
/// Conservative by design: only marks targets as "safe" when trust is high
/// and reverse impact is empty or explicitly ignorable.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
pub fn safe_delete(
    cwd: &Path,
    config: &PruneguardConfig,
    targets: &[String],
    profile: EntrypointProfile,
    options: &SafeDeleteOptions,
) -> miette::Result<SafeDeleteReport> {
    let scan_options = ScanOptions {
        config_dir: options.config_dir.clone(),
        no_cache: options.no_cache,
        no_baseline: true,
        require_full_scope: false,
        ..ScanOptions::default()
    };
    let execution = scan_with_options(cwd, config, &[], profile, &scan_options)?;
    let report = &execution.report;

    if report.stats.partial_scope {
        return Ok(SafeDeleteReport {
            targets: targets.to_vec(),
            safe: Vec::new(),
            needs_review: Vec::new(),
            blocked: targets
                .iter()
                .map(|target| SafeDeleteCandidate {
                    target: target.clone(),
                    classification: SafeDeleteClassification::Blocked,
                    confidence: None,
                    reasons: vec![
                        "Partial-scope analysis was used. Full-scope is required for safe-delete."
                            .to_string(),
                    ],
                    evidence: Vec::new(),
                })
                .collect(),
            deletion_order: Vec::new(),
            evidence: Vec::new(),
        });
    }

    let unresolved_pressure = if report.stats.files_resolved > 0 {
        report.stats.unresolved_specifiers as f64
            / (report.stats.files_resolved + report.stats.unresolved_specifiers) as f64
    } else {
        0.0
    };
    let high_pressure = unresolved_pressure > 0.05;

    let finding_subjects: BTreeSet<String> =
        report.findings.iter().map(|f| f.subject.clone()).collect();

    let mut safe = Vec::new();
    let mut needs_review = Vec::new();
    let mut blocked = Vec::new();
    let mut deletion_order_targets: Vec<String> = Vec::new();
    let mut all_evidence = Vec::new();

    for target in targets {
        let is_finding = finding_subjects.contains(target)
            || report.findings.iter().any(|f| {
                f.subject.starts_with(target) || f.subject.split('#').next() == Some(target)
            });

        if !is_finding {
            blocked.push(SafeDeleteCandidate {
                target: target.clone(),
                classification: SafeDeleteClassification::Blocked,
                confidence: None,
                reasons: vec![format!("`{target}` was not flagged as unused by the analysis.")],
                evidence: vec![pruneguard_report::Evidence {
                    kind: "absence".to_string(),
                    file: Some(target.clone()),
                    line: None,
                    description: "No unused-file or unused-export finding matches this target."
                        .to_string(),
                }],
            });
            continue;
        }

        let related_findings: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| {
                f.subject == *target
                    || f.subject.starts_with(target)
                    || f.subject.split('#').next() == Some(target.as_str())
            })
            .collect();

        let impact_result =
            impact_with_options(cwd, config, target, profile, &ImpactOptions::default());

        let has_reverse_impact = match &impact_result {
            Ok(ir) => {
                // Exclude the target itself from affected-files check.
                let other_affected = ir
                    .affected_files
                    .iter()
                    .any(|f| f != target && !f.ends_with(target) && !target.ends_with(f));
                !ir.affected_entrypoints.is_empty() || other_affected
            }
            Err(_) => false,
        };

        if has_reverse_impact {
            let mut reasons = vec![format!(
                "`{target}` has reverse impact — other files or entrypoints depend on it."
            )];
            let mut candidate_evidence = Vec::new();
            if let Ok(ir) = &impact_result {
                if !ir.affected_entrypoints.is_empty() {
                    reasons.push(format!(
                        "Affected entrypoints: {}",
                        ir.affected_entrypoints
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let impact_evidence = pruneguard_report::Evidence {
                    kind: "impact".to_string(),
                    file: Some(target.clone()),
                    line: None,
                    description: format!(
                        "{} affected entrypoints, {} affected files",
                        ir.affected_entrypoints.len(),
                        ir.affected_files.len()
                    ),
                };
                candidate_evidence.push(impact_evidence.clone());
                all_evidence.push(impact_evidence);
            }
            blocked.push(SafeDeleteCandidate {
                target: target.clone(),
                classification: SafeDeleteClassification::Blocked,
                confidence: None,
                reasons,
                evidence: candidate_evidence,
            });
            continue;
        }

        let all_high_confidence =
            related_findings.iter().all(|f| f.confidence == FindingConfidence::High);
        let any_low_confidence =
            related_findings.iter().any(|f| f.confidence == FindingConfidence::Low);

        // Collect evidence from related findings for all candidates.
        let candidate_evidence: Vec<pruneguard_report::Evidence> = related_findings
            .iter()
            .map(|f| pruneguard_report::Evidence {
                kind: "finding".to_string(),
                file: Some(f.subject.clone()),
                line: None,
                description: format!("[{}] {} (confidence: {:?})", f.code, f.message, f.confidence),
            })
            .collect();

        if high_pressure {
            needs_review.push(SafeDeleteCandidate {
                target: target.clone(),
                classification: SafeDeleteClassification::NeedsReview,
                confidence: Some(FindingConfidence::Low),
                reasons: vec![
                    format!(
                        "Unresolved pressure is {:.1}% — cannot guarantee safety.",
                        unresolved_pressure * 100.0
                    ),
                    "Resolve unresolved specifiers and re-run safe-delete for a definitive answer."
                        .to_string(),
                ],
                evidence: candidate_evidence,
            });
        } else if any_low_confidence {
            needs_review.push(SafeDeleteCandidate {
                target: target.clone(),
                classification: SafeDeleteClassification::NeedsReview,
                confidence: Some(FindingConfidence::Low),
                reasons: vec![
                    "At least one related finding has low confidence — manual review required."
                        .to_string(),
                    "Consider running `pruneguard explain` on the target for more context."
                        .to_string(),
                ],
                evidence: candidate_evidence,
            });
        } else if all_high_confidence {
            safe.push(SafeDeleteCandidate {
                target: target.clone(),
                classification: SafeDeleteClassification::Safe,
                confidence: Some(FindingConfidence::High),
                reasons: vec![
                    "No reverse impact detected and all related findings have high confidence."
                        .to_string(),
                ],
                evidence: candidate_evidence,
            });
            deletion_order_targets.push(target.clone());
        } else {
            needs_review.push(SafeDeleteCandidate {
                target: target.clone(),
                classification: SafeDeleteClassification::NeedsReview,
                confidence: Some(FindingConfidence::Medium),
                reasons: vec![
                    "Finding confidence is not uniformly high — manual review recommended."
                        .to_string(),
                ],
                evidence: candidate_evidence,
            });
        }
    }

    // --- Trust-aware downgrades for Safe candidates ---
    //
    // Even when the base classification is Safe, trust signals may lower
    // confidence enough to warrant manual review.
    let compat = compute_compat_report_from_build(&execution.build);

    #[allow(clippy::cast_precision_loss)]
    let unresolved_pressure_high = unresolved_pressure > 0.10;

    let mut downgraded_safe = Vec::new();
    safe.retain(|candidate| {
        // Check: compatibility trust downgrades affect this target
        if compat.is_path_affected(&candidate.target) {
            let trust_reason = compat
                .trust_notes_for_path(&candidate.target)
                .first()
                .cloned()
                .unwrap_or_else(|| "compatibility trust downgrade".to_string());
            downgraded_safe.push(SafeDeleteCandidate {
                target: candidate.target.clone(),
                classification: SafeDeleteClassification::NeedsReview,
                confidence: Some(FindingConfidence::Medium),
                reasons: vec![trust_reason],
                evidence: candidate.evidence.clone(),
            });
            deletion_order_targets.retain(|t| t != &candidate.target);
            return false;
        }

        // Check: high unresolved pressure (>10%)
        if unresolved_pressure_high {
            downgraded_safe.push(SafeDeleteCandidate {
                target: candidate.target.clone(),
                classification: SafeDeleteClassification::NeedsReview,
                confidence: Some(FindingConfidence::Medium),
                reasons: vec!["high unresolved specifier pressure".to_string()],
                evidence: candidate.evidence.clone(),
            });
            deletion_order_targets.retain(|t| t != &candidate.target);
            return false;
        }

        true
    });
    needs_review.extend(downgraded_safe);

    // Compute a stable deletion order: files with fewer dependencies on other
    // targets (leaves) should be deleted first to avoid breaking intermediate
    // references during batch deletion.
    compute_deletion_order(&mut deletion_order_targets, &execution.build);

    let deletion_order: Vec<DeletionOrderEntry> = deletion_order_targets
        .iter()
        .enumerate()
        .map(|(i, target)| {
            let reason = if deletion_order_targets.len() <= 1 {
                None
            } else if i == 0 {
                Some("Leaf target — no other safe targets depend on it.".to_string())
            } else {
                Some(format!("Depends on {i} other safe target(s) — delete after them."))
            };
            DeletionOrderEntry { target: target.clone(), step: i + 1, reason }
        })
        .collect();

    Ok(SafeDeleteReport {
        targets: targets.to_vec(),
        safe,
        needs_review,
        blocked,
        deletion_order,
        evidence: all_evidence,
    })
}

/// Generate a fix plan for the given targets.
///
/// Matches targets against findings by ID, file path, or export name,
/// then generates remediation actions for each matched finding.
#[allow(clippy::too_many_lines)]
pub fn fix_plan(
    cwd: &Path,
    config: &PruneguardConfig,
    targets: &[String],
    profile: EntrypointProfile,
    options: &FixPlanOptions,
) -> miette::Result<FixPlanReport> {
    let scan_options = ScanOptions {
        config_dir: options.config_dir.clone(),
        no_cache: options.no_cache,
        no_baseline: true,
        require_full_scope: false,
        ..ScanOptions::default()
    };
    let execution = scan_with_options(cwd, config, &[], profile, &scan_options)?;
    let report = &execution.report;

    let matched_findings: Vec<Finding> = report
        .findings
        .iter()
        .filter(|finding| {
            targets.iter().any(|target| {
                finding.id == *target
                    || finding.subject == *target
                    || finding.subject.starts_with(target)
                    || finding.subject.split('#').next() == Some(target.as_str())
            })
        })
        .cloned()
        .collect();

    let mut actions = Vec::new();
    let mut blocked_by = Vec::new();

    for finding in &matched_findings {
        let kind = match finding.code.as_str() {
            "unused-file" => RemediationActionKind::DeleteFile,
            "unused-export" => RemediationActionKind::DeleteExport,
            "unused-dependency" | "unused-package" => RemediationActionKind::RemoveDependency,
            "cycle" => RemediationActionKind::BreakCycle,
            "boundary-violation" => RemediationActionKind::UpdateBoundaryRule,
            "ownership-unowned" | "ownership-cross-owner" | "ownership-hotspot" => {
                RemediationActionKind::AssignOwner
            }
            _ => {
                blocked_by
                    .push(format!("No remediation strategy for finding code `{}`.", finding.code));
                continue;
            }
        };

        let steps = generate_remediation_steps(kind, &finding.subject);
        let preconditions = generate_preconditions(kind, &finding.subject);
        let verification = generate_verification_steps(kind, &finding.subject);

        actions.push(RemediationAction {
            id: format!("fix-{}", finding.id),
            kind,
            targets: vec![finding.subject.clone()],
            why: finding.message.clone(),
            preconditions,
            steps,
            verification,
            risk: risk_for_action(kind),
            confidence: finding.confidence,
            rank: None,
            phase: Some(phase_name_for_action(kind).to_string()),
            finding_ids: vec![finding.id.clone()],
        });
    }

    // --- Trust-aware confidence adjustments ---
    //
    // If the scan conditions reduce trust, lower the confidence of each
    // remediation action so that agents treat the plan more cautiously.
    let is_partial_scope = report.stats.partial_scope;
    let has_heuristic_framework = !report.stats.heuristic_frameworks.is_empty();

    if is_partial_scope || has_heuristic_framework {
        for action in &mut actions {
            if is_partial_scope {
                action.confidence = lower_confidence(action.confidence);
            }
            if has_heuristic_framework {
                // Check if any heuristic framework is relevant to this action's targets.
                let heuristic_affects_target = action.targets.iter().any(|target| {
                    report.findings.iter().any(|f| {
                        (f.subject == *target
                            || f.subject.starts_with(target)
                            || f.subject.split('#').next() == Some(target.as_str()))
                            && f.framework_context.is_some()
                    })
                });
                if heuristic_affects_target {
                    action.confidence = lower_confidence(action.confidence);
                }
            }
        }
    }

    // Apply ranking policy: minimal blast radius first, high confidence first,
    // low unresolved pressure first, boundary/ownership fixes after dead-code
    // cleanup.
    rank_remediation_actions(&mut actions);

    let risk_level = actions
        .iter()
        .map(|action| action.risk)
        .max_by_key(|risk| match risk {
            RiskLevel::Low => 0,
            RiskLevel::Medium => 1,
            RiskLevel::High => 2,
        })
        .unwrap_or(RiskLevel::Low);

    let confidence = matched_findings
        .iter()
        .map(|finding| finding.confidence)
        .min_by_key(|confidence| match confidence {
            FindingConfidence::High => 0,
            FindingConfidence::Medium => 1,
            FindingConfidence::Low => 2,
        })
        .unwrap_or(FindingConfidence::High);

    let total_actions = actions.len();
    let high_confidence_actions =
        actions.iter().filter(|a| matches!(a.confidence, FindingConfidence::High)).count();

    // Build phase summary.
    let phase_summary = build_phase_summary(&actions);

    let verification_steps = vec![
        "Run `pruneguard scan` to verify all findings are resolved.".to_string(),
        "Run your test suite to confirm no regressions.".to_string(),
        "Run `pruneguard review` to confirm no new blocking findings.".to_string(),
    ];

    Ok(FixPlanReport {
        query: targets.to_vec(),
        matched_findings,
        actions,
        blocked_by,
        verification_steps,
        risk_level,
        confidence,
        total_actions,
        high_confidence_actions,
        phase_summary,
    })
}

/// Suggest governance rules based on graph analysis.
pub fn suggest_rules(
    cwd: &Path,
    config: &PruneguardConfig,
    profile: EntrypointProfile,
    options: &SuggestRulesOptions,
) -> miette::Result<SuggestRulesReport> {
    let scan_options = ScanOptions {
        config_dir: options.config_dir.clone(),
        no_cache: options.no_cache,
        no_baseline: true,
        require_full_scope: false,
        ..ScanOptions::default()
    };
    let execution = scan_with_options(cwd, config, &[], profile, &scan_options)?;
    Ok(pruneguard_analyzers::suggest_rules::suggest_rules(&execution.build, config))
}

fn generate_remediation_steps(kind: RemediationActionKind, subject: &str) -> Vec<RemediationStep> {
    let file_path = subject.split('#').next().unwrap_or(subject);
    match kind {
        RemediationActionKind::DeleteFile => vec![RemediationStep {
            description: format!("Delete the file `{file_path}`."),
            file: Some(file_path.to_string()),
            action: Some("delete".to_string()),
        }],
        RemediationActionKind::DeleteExport => vec![RemediationStep {
            description: format!("Remove the unused export from `{subject}`."),
            file: Some(file_path.to_string()),
            action: Some("edit".to_string()),
        }],
        RemediationActionKind::RemoveDependency => vec![RemediationStep {
            description: format!("Remove `{subject}` from package.json dependencies."),
            file: None,
            action: Some("edit".to_string()),
        }],
        RemediationActionKind::BreakCycle => vec![
            RemediationStep {
                description: format!(
                    "Identify the weakest edge in the cycle involving `{subject}`."
                ),
                file: Some(file_path.to_string()),
                action: None,
            },
            RemediationStep {
                description: "Extract the shared dependency or invert the dependency direction."
                    .to_string(),
                file: None,
                action: Some("refactor".to_string()),
            },
        ],
        RemediationActionKind::UpdateBoundaryRule => vec![RemediationStep {
            description: format!(
                "Update boundary rules or refactor the import involving `{subject}`."
            ),
            file: Some(file_path.to_string()),
            action: Some("edit".to_string()),
        }],
        RemediationActionKind::AssignOwner => vec![RemediationStep {
            description: format!("Assign an owner for `{subject}` in the ownership config."),
            file: None,
            action: Some("config".to_string()),
        }],
        RemediationActionKind::MoveImport
        | RemediationActionKind::TightenEntrypoint
        | RemediationActionKind::SplitPackage
        | RemediationActionKind::AcknowledgeBaseline => vec![RemediationStep {
            description: format!("Apply the `{kind:?}` action for `{subject}`."),
            file: Some(file_path.to_string()),
            action: None,
        }],
    }
}

const fn risk_for_action(kind: RemediationActionKind) -> RiskLevel {
    match kind {
        RemediationActionKind::DeleteFile | RemediationActionKind::RemoveDependency => {
            RiskLevel::Medium
        }
        RemediationActionKind::BreakCycle | RemediationActionKind::SplitPackage => RiskLevel::High,
        RemediationActionKind::DeleteExport
        | RemediationActionKind::MoveImport
        | RemediationActionKind::TightenEntrypoint
        | RemediationActionKind::UpdateBoundaryRule
        | RemediationActionKind::AssignOwner
        | RemediationActionKind::AcknowledgeBaseline => RiskLevel::Low,
    }
}

/// Generate preconditions for a given remediation action kind.
fn generate_preconditions(kind: RemediationActionKind, subject: &str) -> Vec<String> {
    let file_path = subject.split('#').next().unwrap_or(subject);
    match kind {
        RemediationActionKind::DeleteFile => vec![
            format!(
                "Verify `{file_path}` is not referenced by dynamic imports or require calls that the analyzer cannot trace."
            ),
            "Confirm no runtime reflection or string-based module loading references this file."
                .to_string(),
        ],
        RemediationActionKind::DeleteExport => vec![
            format!(
                "Verify the export in `{subject}` is not consumed through re-export barrel files the analyzer may not fully resolve."
            ),
            "Check that no test files outside the analysis scope depend on this export."
                .to_string(),
        ],
        RemediationActionKind::RemoveDependency => vec![
            format!(
                "Verify `{subject}` is not loaded by a build tool, test runner, or script runner that bypasses module imports."
            ),
            "Check scripts in package.json for indirect references.".to_string(),
        ],
        RemediationActionKind::BreakCycle => vec![
            format!(
                "Identify which edge in the cycle involving `{subject}` is the weakest (least semantic coupling)."
            ),
            "Ensure the refactor does not introduce a new cycle elsewhere.".to_string(),
        ],
        RemediationActionKind::UpdateBoundaryRule => vec![
            format!(
                "Review whether the boundary violation for `{subject}` represents an intentional architectural exception."
            ),
            "Consider whether the rule itself needs updating rather than the code.".to_string(),
        ],
        RemediationActionKind::AssignOwner => vec![
            format!("Identify the team or individual responsible for `{subject}`."),
            "Check CODEOWNERS file or git blame for historical ownership signals.".to_string(),
        ],
        RemediationActionKind::MoveImport
        | RemediationActionKind::TightenEntrypoint
        | RemediationActionKind::SplitPackage
        | RemediationActionKind::AcknowledgeBaseline => {
            vec![format!("Review the current state of `{subject}` before applying the action.")]
        }
    }
}

/// Generate verification steps for a given remediation action kind.
fn generate_verification_steps(kind: RemediationActionKind, subject: &str) -> Vec<String> {
    let mut steps = vec![format!(
        "Run `pruneguard scan` and verify `{subject}` no longer appears in findings."
    )];

    match kind {
        RemediationActionKind::DeleteFile => {
            steps.push("Run your test suite to confirm no import errors.".to_string());
            steps.push("Run your build to confirm no compilation errors.".to_string());
        }
        RemediationActionKind::DeleteExport => {
            steps.push(
                "Run your TypeScript compiler (tsc --noEmit) to verify no type errors.".to_string(),
            );
            steps.push("Run your test suite to confirm no regressions.".to_string());
        }
        RemediationActionKind::RemoveDependency => {
            steps.push("Run `npm install` / `pnpm install` to update the lockfile.".to_string());
            steps.push("Run your build and test suite to confirm no missing modules.".to_string());
        }
        RemediationActionKind::BreakCycle => {
            steps.push("Run `pruneguard scan` and verify the cycle finding is gone.".to_string());
            steps.push(
                "Run `pruneguard impact <refactored-file>` to confirm blast radius is acceptable."
                    .to_string(),
            );
        }
        RemediationActionKind::UpdateBoundaryRule => {
            steps.push(
                "Run `pruneguard scan` and verify no boundary violations remain.".to_string(),
            );
        }
        RemediationActionKind::AssignOwner => {
            steps.push(
                "Run `pruneguard scan` and verify no ownership findings remain for this path."
                    .to_string(),
            );
        }
        _ => {
            steps.push("Run your test suite to confirm no regressions.".to_string());
        }
    }

    steps
}

/// Rank remediation actions according to the plan ranking policy:
/// 1. Minimal blast radius first (delete-export < delete-file < remove-dep < break-cycle)
/// 2. High confidence first
/// 3. Low risk first
/// 4. Boundary/ownership fixes after dead-code cleanup
fn rank_remediation_actions(actions: &mut [RemediationAction]) {
    actions.sort_by(|a, b| {
        // Phase ordering: dead-code cleanup before governance fixes
        let phase_a = action_phase(a.kind);
        let phase_b = action_phase(b.kind);
        phase_a
            .cmp(&phase_b)
            // Within same phase: minimal blast radius first
            .then_with(|| blast_radius_rank(a.kind).cmp(&blast_radius_rank(b.kind)))
            // Then high confidence first
            .then_with(|| confidence_rank(a.confidence).cmp(&confidence_rank(b.confidence)))
            // Then low risk first
            .then_with(|| risk_rank(a.risk).cmp(&risk_rank(b.risk)))
    });

    // Assign rank numbers after sorting.
    for (i, action) in actions.iter_mut().enumerate() {
        action.rank = Some(i + 1);
    }
}

/// Return a phase number for ordering: dead-code cleanup (0) before governance (1).
const fn action_phase(kind: RemediationActionKind) -> u8 {
    match kind {
        RemediationActionKind::DeleteExport
        | RemediationActionKind::DeleteFile
        | RemediationActionKind::RemoveDependency => 0,
        RemediationActionKind::BreakCycle
        | RemediationActionKind::MoveImport
        | RemediationActionKind::TightenEntrypoint
        | RemediationActionKind::SplitPackage => 1,
        RemediationActionKind::UpdateBoundaryRule
        | RemediationActionKind::AssignOwner
        | RemediationActionKind::AcknowledgeBaseline => 2,
    }
}

/// Blast radius rank: lower number = smaller blast radius = do first.
const fn blast_radius_rank(kind: RemediationActionKind) -> u8 {
    match kind {
        RemediationActionKind::DeleteExport => 0,
        RemediationActionKind::DeleteFile => 1,
        RemediationActionKind::RemoveDependency => 2,
        RemediationActionKind::AcknowledgeBaseline => 3,
        RemediationActionKind::UpdateBoundaryRule
        | RemediationActionKind::AssignOwner
        | RemediationActionKind::TightenEntrypoint
        | RemediationActionKind::MoveImport => 4,
        RemediationActionKind::BreakCycle => 5,
        RemediationActionKind::SplitPackage => 6,
    }
}

const fn confidence_rank(confidence: FindingConfidence) -> u8 {
    match confidence {
        FindingConfidence::High => 0,
        FindingConfidence::Medium => 1,
        FindingConfidence::Low => 2,
    }
}

/// Lower confidence by one level (High -> Medium -> Low -> Low).
const fn lower_confidence(confidence: FindingConfidence) -> FindingConfidence {
    match confidence {
        FindingConfidence::High => FindingConfidence::Medium,
        FindingConfidence::Medium | FindingConfidence::Low => FindingConfidence::Low,
    }
}

const fn risk_rank(risk: RiskLevel) -> u8 {
    match risk {
        RiskLevel::Low => 0,
        RiskLevel::Medium => 1,
        RiskLevel::High => 2,
    }
}

/// Return a human-readable phase name for a remediation action kind.
const fn phase_name_for_action(kind: RemediationActionKind) -> &'static str {
    match kind {
        RemediationActionKind::DeleteExport
        | RemediationActionKind::DeleteFile
        | RemediationActionKind::RemoveDependency => "dead-code",
        RemediationActionKind::BreakCycle
        | RemediationActionKind::MoveImport
        | RemediationActionKind::TightenEntrypoint
        | RemediationActionKind::SplitPackage => "architecture",
        RemediationActionKind::UpdateBoundaryRule
        | RemediationActionKind::AssignOwner
        | RemediationActionKind::AcknowledgeBaseline => "governance",
    }
}

/// Build a summary of actions grouped by phase.
fn build_phase_summary(actions: &[RemediationAction]) -> Vec<FixPlanPhase> {
    let mut phases: Vec<FixPlanPhase> = Vec::new();

    let phase_defs: &[(&str, usize, &str)] = &[
        ("dead-code", 0, "Remove unused files, exports, and dependencies."),
        ("architecture", 1, "Break cycles, move imports, and restructure packages."),
        ("governance", 2, "Update boundary rules, assign owners, and acknowledge baselines."),
    ];

    for &(name, order, description) in phase_defs {
        let count = actions.iter().filter(|a| a.phase.as_deref() == Some(name)).count();
        if count > 0 {
            phases.push(FixPlanPhase {
                name: name.to_string(),
                order,
                action_count: count,
                description: description.to_string(),
            });
        }
    }

    phases
}

/// Compute a stable deletion order for safe-delete targets.
///
/// Files that are depended upon by other safe-delete targets should be deleted
/// last — so we delete "importers first, leaves last". This prevents broken
/// intermediate references during batch deletion. Within the same dependency
/// depth, we sort alphabetically for determinism.
fn compute_deletion_order(order: &mut Vec<String>, build: &GraphBuildResult) {
    use petgraph::visit::EdgeRef;

    let safe_set: BTreeSet<&str> = order.iter().map(String::as_str).collect();
    if safe_set.len() <= 1 {
        return;
    }

    // For each target, count how many other safe-set targets import it (reverse
    // edges within the safe set). Targets with zero reverse-safe-set edges are
    // "leaves" — nothing in the safe set depends on them. We delete non-leaves
    // (importers) first, leaves last.
    let mut reverse_counts: Vec<(String, usize)> = order
        .iter()
        .map(|target| {
            let count = build
                .find_file(target)
                .and_then(|file| {
                    let file_id = build.module_graph.file_id(&file.file.path.to_string_lossy())?;
                    let (node_index, _) = build.module_graph.file_node_by_id(file_id)?;
                    // Count incoming edges from other safe-set files.
                    Some(
                        build
                            .module_graph
                            .graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .filter(|edge| {
                                if let ModuleNode::File { relative_path, .. } =
                                    &build.module_graph.graph[edge.source()]
                                {
                                    safe_set.contains(relative_path.as_str())
                                } else {
                                    false
                                }
                            })
                            .count(),
                    )
                })
                .unwrap_or(0);
            (target.clone(), count)
        })
        .collect();

    // Sort: targets with the most reverse-safe-set edges (most depended upon)
    // go last. Targets depended on by nothing in the safe set go first.
    // Tie-break alphabetically for determinism.
    reverse_counts.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    *order = reverse_counts.into_iter().map(|(target, _)| target).collect();
}

/// Debug: resolve a specifier from a file.
pub fn debug_resolve(
    cwd: &Path,
    config: &PruneguardConfig,
    specifier: &str,
    from: &Path,
) -> String {
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
#[napi(object)]
pub struct JsDebugFrameworksOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsCompatibilityReportOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
}

#[cfg(feature = "napi")]
#[napi]
pub fn debug_frameworks_json(options: JsDebugFrameworksOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let report = debug_frameworks(&cwd, parse_profile(options.profile.as_deref()))
        .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn compatibility_report_json(options: JsCompatibilityReportOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let profile = EntrypointProfile::Both; // compatibility-report uses all profiles
    let report = compatibility_report(&cwd, profile)
        .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
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
