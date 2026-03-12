use pruneguard_config::{AnalysisSeverity, PruneguardConfig};
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_graph::GraphBuildResult;
use pruneguard_report::{Finding, FindingSeverity};
use pruneguard_rules::CompiledRules;

/// Evaluate compiled forbidden dependency rules.
pub fn analyze(
    build: &GraphBuildResult,
    config: &PruneguardConfig,
    level: AnalysisSeverity,
    profile: EntrypointProfile,
    rules: &CompiledRules,
) -> Vec<Finding> {
    rules
        .evaluate(build, config, profile)
        .into_iter()
        .filter(|finding| severity_at_or_above(level, finding.severity))
        .collect()
}

const fn severity_at_or_above(level: AnalysisSeverity, finding: FindingSeverity) -> bool {
    match level {
        AnalysisSeverity::Off => false,
        AnalysisSeverity::Info => true,
        AnalysisSeverity::Warn => {
            matches!(finding, FindingSeverity::Error | FindingSeverity::Warn)
        }
        AnalysisSeverity::Error => matches!(finding, FindingSeverity::Error),
    }
}
