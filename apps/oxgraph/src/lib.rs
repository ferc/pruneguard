use std::path::Path;

use oxgraph_config::OxgraphConfig;
use oxgraph_report::{AnalysisReport, ExplainReport, ImpactReport};

/// Run a full scan and return the analysis report.
pub fn scan(
    _cwd: &Path,
    _config: &OxgraphConfig,
    _paths: &[std::path::PathBuf],
) -> miette::Result<AnalysisReport> {
    miette::bail!("scan is not yet implemented")
}

/// Compute the blast radius for a target.
pub fn impact(_cwd: &Path, _config: &OxgraphConfig, _target: &str) -> miette::Result<ImpactReport> {
    miette::bail!("impact is not yet implemented")
}

/// Explain a finding or path.
pub fn explain(
    _cwd: &Path,
    _config: &OxgraphConfig,
    _query: &str,
) -> miette::Result<ExplainReport> {
    miette::bail!("explain is not yet implemented")
}

/// Debug: list all detected entrypoints.
pub fn debug_entrypoints(_cwd: &Path, _config: &OxgraphConfig) -> miette::Result<Vec<String>> {
    miette::bail!("debug entrypoints is not yet implemented")
}
