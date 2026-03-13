pub mod boundaries;
pub mod cycles;
pub mod duplicate_exports;
pub mod external_parity;
pub mod impact;
pub mod ownership;
pub mod parity;
pub mod semantic_scheduler;
pub mod suggest_rules;
pub mod unused_dependencies;
pub mod unused_exports;
pub mod unused_files;
pub mod unused_members;
pub mod unused_packages;

use std::hash::{Hash, Hasher};

use globset::{Glob, GlobSet, GlobSetBuilder};
use pruneguard_config::{AnalysisSeverity, IgnoreIssueRule, PruneguardConfig};
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_graph::GraphBuildResult;
use pruneguard_report::{
    Evidence, Finding, FindingCategory, FindingConfidence, FindingSeverity, RemediationActionKind,
};
use pruneguard_rules::CompiledRules;

/// Run all enabled analyzers and collect findings.
///
/// Returns the filtered findings and the count of findings suppressed by
/// `ignoreIssues` rules.
pub fn run_analyzers(
    build: &GraphBuildResult,
    config: &PruneguardConfig,
    profile: EntrypointProfile,
) -> (Vec<Finding>, usize) {
    let mut findings = Vec::new();

    let reachable = build.module_graph.reachable_file_ids(profile);
    let reachable_prod = if profile == EntrypointProfile::Production {
        reachable.clone()
    } else {
        build.module_graph.reachable_file_ids(EntrypointProfile::Production)
    };
    let reachable_dev = if profile == EntrypointProfile::Development {
        reachable.clone()
    } else {
        build.module_graph.reachable_file_ids(EntrypointProfile::Development)
    };

    findings.extend(unused_files::analyze(
        build,
        config.analysis.unused_files,
        profile,
        &reachable,
    ));
    findings.extend(unused_exports::analyze(
        build,
        config.analysis.unused_exports,
        profile,
        config.analysis.ignore_exports_used_in_file,
        &reachable,
    ));
    findings.extend(unused_dependencies::analyze(
        build,
        config.analysis.unused_dependencies,
        profile,
        &reachable_prod,
        &reachable_dev,
    ));
    findings.extend(unused_packages::analyze(build, config.analysis.unused_packages, profile));
    findings.extend(cycles::analyze(build, config.analysis.cycles));

    if let Some(rules) = &config.rules
        && config.analysis.boundaries != AnalysisSeverity::Off
        && let Ok(compiled) = CompiledRules::compile(rules)
    {
        findings.extend(boundaries::analyze(
            build,
            config,
            config.analysis.boundaries,
            profile,
            &compiled,
        ));
    }

    findings.extend(ownership::analyze(
        build,
        config.ownership.as_ref(),
        config.analysis.ownership,
    ));

    findings.extend(unused_members::analyze(build, &config.analysis));
    findings.extend(duplicate_exports::analyze(build, config.analysis.duplicate_exports));

    // Apply ignore_issues rules to suppress matching findings.
    let suppressed = apply_ignore_issues(&mut findings, &config.ignore_issues);
    if suppressed > 0 {
        tracing::info!(suppressed, "findings suppressed by ignoreIssues rules");
    }

    findings.sort_by(|left, right| left.id.cmp(&right.id));
    (findings, suppressed)
}

pub(crate) const fn severity(level: AnalysisSeverity) -> Option<FindingSeverity> {
    match level {
        AnalysisSeverity::Off => None,
        AnalysisSeverity::Info => Some(FindingSeverity::Info),
        AnalysisSeverity::Warn => Some(FindingSeverity::Warn),
        AnalysisSeverity::Error => Some(FindingSeverity::Error),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn make_finding(
    code: &str,
    severity: FindingSeverity,
    category: FindingCategory,
    confidence: FindingConfidence,
    subject: impl Into<String>,
    workspace: Option<String>,
    package: Option<String>,
    message: impl Into<String>,
    evidence: Vec<Evidence>,
    suggestion: Option<String>,
    rule_name: Option<String>,
) -> Finding {
    let subject = subject.into();
    let message = message.into();
    let primary_evidence = evidence.first().map_or(String::new(), |item| {
        format!("{}|{}|{}", item.kind, item.file.clone().unwrap_or_default(), item.description)
    });
    let id = finding_id(code, &subject, rule_name.as_deref(), &primary_evidence);
    let primary_action_kind = primary_action_kind_for_code(code);
    let action_kinds = primary_action_kind.map_or_else(Vec::new, |kind| vec![kind]);

    Finding {
        id,
        code: code.to_string(),
        severity,
        category,
        confidence,
        subject,
        workspace,
        package,
        message,
        evidence,
        suggestion,
        rule_name,
        primary_action_kind,
        action_kinds,
        trust_notes: None,
        framework_context: None,
        precision_source: None,
        confidence_reason: None,
    }
}

/// Map a finding code to its primary remediation action kind.
pub fn primary_action_kind_for_code(code: &str) -> Option<RemediationActionKind> {
    match code {
        "unused-file" => Some(RemediationActionKind::DeleteFile),
        "unused-export" => Some(RemediationActionKind::DeleteExport),
        "unused-dependency" | "unused-package" => Some(RemediationActionKind::RemoveDependency),
        "cycle" => Some(RemediationActionKind::BreakCycle),
        "boundary-violation" => Some(RemediationActionKind::UpdateBoundaryRule),
        "ownership-unowned" | "ownership-cross-owner" | "ownership-hotspot" => {
            Some(RemediationActionKind::AssignOwner)
        }
        _ => None,
    }
}

fn finding_id(
    code: &str,
    subject: &str,
    rule_name: Option<&str>,
    primary_evidence: &str,
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    subject.hash(&mut hasher);
    rule_name.hash(&mut hasher);
    primary_evidence.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Convert a camelCase string to kebab-case.
///
/// For example, `"unusedExport"` becomes `"unused-export"`.
fn camel_to_kebab(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// A compiled ignore-issue rule: a normalized kind and an optional glob set for file matching.
struct CompiledIgnoreRule {
    /// Normalized (kebab-case) finding kind.
    kind: String,
    /// When present, only suppress findings whose `subject` matches one of the globs.
    file_matcher: Option<GlobSet>,
}

/// Compile `IgnoreIssueRule` entries into matchers, skipping any with invalid globs.
fn compile_ignore_rules(rules: &[IgnoreIssueRule]) -> Vec<CompiledIgnoreRule> {
    rules
        .iter()
        .filter_map(|rule| {
            let kind = if rule.kind.contains('-') {
                rule.kind.clone()
            } else {
                camel_to_kebab(&rule.kind)
            };

            let file_matcher = if rule.files.is_empty() {
                None
            } else {
                let mut builder = GlobSetBuilder::new();
                for pattern in &rule.files {
                    match Glob::new(pattern) {
                        Ok(glob) => {
                            builder.add(glob);
                        }
                        Err(err) => {
                            tracing::warn!(
                                pattern,
                                %err,
                                "invalid glob in ignoreIssues rule, skipping pattern"
                            );
                        }
                    }
                }
                match builder.build() {
                    Ok(set) => Some(set),
                    Err(err) => {
                        tracing::warn!(
                            kind,
                            %err,
                            "failed to compile glob set for ignoreIssues rule, skipping rule"
                        );
                        return None;
                    }
                }
            };

            Some(CompiledIgnoreRule { kind, file_matcher })
        })
        .collect()
}

/// Remove findings that match any `ignore_issues` rule and return the number of
/// suppressed findings.
fn apply_ignore_issues(findings: &mut Vec<Finding>, rules: &[IgnoreIssueRule]) -> usize {
    if rules.is_empty() {
        return 0;
    }

    let compiled = compile_ignore_rules(rules);
    if compiled.is_empty() {
        return 0;
    }

    let before = findings.len();
    findings.retain(|finding| {
        !compiled.iter().any(|rule| {
            if finding.code != rule.kind {
                return false;
            }
            match &rule.file_matcher {
                None => true,
                Some(glob_set) => glob_set.is_match(&finding.subject),
            }
        })
    });
    before - findings.len()
}
