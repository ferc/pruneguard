pub mod boundaries;
pub mod cycles;
pub mod impact;
pub mod ownership;
pub mod unused_dependencies;
pub mod unused_exports;
pub mod unused_files;
pub mod unused_packages;

use std::hash::{Hash, Hasher};

use oxgraph_config::{AnalysisSeverity, OxgraphConfig};
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Evidence, Finding, FindingCategory, FindingSeverity};
use oxgraph_rules::CompiledRules;

/// Run all enabled analyzers and collect findings.
pub fn run_analyzers(
    build: &GraphBuildResult,
    config: &OxgraphConfig,
    profile: EntrypointProfile,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    findings.extend(unused_files::analyze(build, config.analysis.unused_files, profile));
    findings.extend(unused_exports::analyze(build, config.analysis.unused_exports, profile));
    findings.extend(unused_dependencies::analyze(
        build,
        config.analysis.unused_dependencies,
        profile,
    ));
    findings.extend(unused_packages::analyze(build, config.analysis.unused_packages, profile));
    findings.extend(cycles::analyze(build, config.analysis.cycles));

    if let Some(rules) = &config.rules
        && config.analysis.boundaries != AnalysisSeverity::Off
        && let Ok(compiled) = CompiledRules::compile(rules)
    {
        findings.extend(boundaries::analyze(
            build,
            config.analysis.boundaries,
            &compiled,
        ));
    }

    findings.extend(ownership::analyze(
        build,
        config.ownership.as_ref(),
        config.analysis.ownership,
    ));

    findings.sort_by(|left, right| left.id.cmp(&right.id));
    findings
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
        format!(
            "{}|{}|{}",
            item.kind,
            item.file.clone().unwrap_or_default(),
            item.description
        )
    });
    let id = finding_id(code, &subject, rule_name.as_deref(), &primary_evidence);

    Finding {
        id,
        code: code.to_string(),
        severity,
        category,
        subject,
        workspace,
        package,
        message,
        evidence,
        suggestion,
        rule_name,
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
