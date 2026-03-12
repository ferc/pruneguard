use oxgraph_config::AnalysisSeverity;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Finding, FindingSeverity};
use oxgraph_rules::CompiledRules;

/// Evaluate compiled forbidden dependency rules.
pub fn analyze(
    build: &GraphBuildResult,
    level: AnalysisSeverity,
    rules: &CompiledRules,
) -> Vec<Finding> {
    rules.evaluate(build)
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
