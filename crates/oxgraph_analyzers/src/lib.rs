// Stub implementations — allow clippy lints that will resolve once analyzers are implemented.
#![allow(clippy::missing_const_for_fn)]

pub mod boundaries;
pub mod cycles;
pub mod impact;
pub mod ownership;
pub mod unused_dependencies;
pub mod unused_exports;
pub mod unused_files;
pub mod unused_packages;

use oxgraph_config::AnalysisConfig;
use oxgraph_graph::ModuleGraph;
use oxgraph_report::Finding;

/// Run all enabled analyzers and collect findings.
pub fn run_analyzers(_module_graph: &ModuleGraph, _config: &AnalysisConfig) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Each analyzer checks its severity in config and skips if "off"
    // TODO: wire up each analyzer

    // Sort findings for deterministic output
    findings.sort_by(|a: &Finding, b: &Finding| a.id.cmp(&b.id));
    findings
}
