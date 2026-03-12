// Stub implementations — allow clippy lints that will resolve once rules are implemented.
#![allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps, clippy::unused_self)]

use oxgraph_config::RulesConfig;
use oxgraph_graph::ModuleGraph;
use oxgraph_report::Finding;

/// Compiled rule set ready for evaluation.
pub struct CompiledRules {
    pub forbidden: Vec<CompiledRule>,
    pub required: Vec<CompiledRule>,
    pub allow: Vec<CompiledRule>,
}

/// A single compiled rule.
pub struct CompiledRule {
    pub name: String,
    pub from_matcher: Option<PathMatcher>,
    pub to_matcher: Option<PathMatcher>,
}

/// Glob-based path matcher for rules.
pub struct PathMatcher {
    pub patterns: Vec<globset::GlobMatcher>,
    pub negative_patterns: Vec<globset::GlobMatcher>,
}

impl CompiledRules {
    /// Compile rules from config.
    pub fn compile(_config: &RulesConfig) -> miette::Result<Self> {
        // TODO: compile glob patterns into matchers
        Ok(Self { forbidden: Vec::new(), required: Vec::new(), allow: Vec::new() })
    }

    /// Evaluate all rules against the graph and return findings.
    pub fn evaluate(&self, _graph: &ModuleGraph) -> Vec<Finding> {
        // TODO: iterate graph edges and check against compiled predicates
        Vec::new()
    }
}
