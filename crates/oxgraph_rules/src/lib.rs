use globset::{Glob, GlobSet, GlobSetBuilder};
use oxgraph_config::{AnalysisSeverity, Rule, RuleFilter, RulesConfig};
use oxgraph_graph::{GraphBuildResult, ModuleEdge, ModuleNode};
use oxgraph_report::{Evidence, Finding, FindingCategory, FindingSeverity};

/// Compiled rule set ready for evaluation.
pub struct CompiledRules {
    pub forbidden: Vec<CompiledRule>,
    pub required: Vec<CompiledRule>,
    pub allow: Vec<CompiledRule>,
}

/// A single compiled rule.
pub struct CompiledRule {
    pub name: String,
    pub severity: AnalysisSeverity,
    pub from_matcher: Option<CompiledFilter>,
    pub to_matcher: Option<CompiledFilter>,
}

/// Compiled file/package/workspace matcher.
pub struct CompiledFilter {
    path: Option<GlobSet>,
    path_not: Option<GlobSet>,
    workspace: Option<GlobSet>,
    workspace_not: Option<GlobSet>,
    package: Option<GlobSet>,
    package_not: Option<GlobSet>,
}

impl CompiledRules {
    /// Compile rules from config.
    pub fn compile(config: &RulesConfig) -> miette::Result<Self> {
        Ok(Self {
            forbidden: config
                .forbidden
                .iter()
                .map(CompiledRule::compile)
                .collect::<miette::Result<_>>()?,
            required: config
                .required
                .iter()
                .map(CompiledRule::compile)
                .collect::<miette::Result<_>>()?,
            allow: config
                .allow
                .iter()
                .map(CompiledRule::compile)
                .collect::<miette::Result<_>>()?,
        })
    }

    /// Evaluate forbidden rules against graph file edges.
    pub fn evaluate(&self, build: &GraphBuildResult) -> Vec<Finding> {
        let mut findings = Vec::new();

        for edge in build.module_graph.graph.edge_indices() {
            let Some((from_index, to_index)) = build.module_graph.graph.edge_endpoints(edge) else {
                continue;
            };
            let edge_weight = build.module_graph.graph[edge];
            if !matches!(
                edge_weight,
                ModuleEdge::StaticImportValue
                    | ModuleEdge::StaticImportType
                    | ModuleEdge::DynamicImport
                    | ModuleEdge::Require
                    | ModuleEdge::SideEffectImport
                    | ModuleEdge::ReExportNamed
                    | ModuleEdge::ReExportAll
            ) {
                continue;
            }

            let ModuleNode::File {
                relative_path: from_path,
                workspace: from_workspace,
                package: from_package,
                ..
            } = &build.module_graph.graph[from_index]
            else {
                continue;
            };

            let (to_path, to_workspace, to_package, subject) = match &build.module_graph.graph[to_index] {
                ModuleNode::File {
                    relative_path,
                    workspace,
                    package,
                    ..
                } => (
                    Some(relative_path.as_str()),
                    workspace.as_deref(),
                    package.as_deref(),
                    relative_path.clone(),
                ),
                ModuleNode::ExternalDependency { name } => (None, None, Some(name.as_str()), name.clone()),
                _ => continue,
            };

            for rule in &self.forbidden {
                let Some(severity) = rule.severity() else {
                    continue;
                };

                if !rule.matches_from(from_path, from_workspace.as_deref(), from_package.as_deref()) {
                    continue;
                }

                if !rule.matches_to(to_path, to_workspace, to_package) {
                    continue;
                }

                findings.push(Finding {
                    id: rule_finding_id(&rule.name, from_path, &subject),
                    code: "boundary-violation".to_string(),
                    severity,
                    category: FindingCategory::BoundaryViolation,
                    subject: format!("{from_path} -> {subject}"),
                    workspace: from_workspace.clone(),
                    package: from_package.clone(),
                    message: format!(
                        "Rule `{}` forbids `{from_path}` from depending on `{subject}`.",
                        rule.name
                    ),
                    evidence: vec![Evidence {
                        kind: "rule".to_string(),
                        file: Some(from_path.clone()),
                        line: None,
                        description: format!("Matched forbidden rule `{}`.", rule.name),
                    }],
                    suggestion: Some("Adjust the dependency direction or narrow the rule scope.".to_string()),
                    rule_name: Some(rule.name.clone()),
                });
            }
        }

        findings.sort_by(|left, right| left.id.cmp(&right.id));
        findings
    }
}

impl CompiledRule {
    fn compile(rule: &Rule) -> miette::Result<Self> {
        Ok(Self {
            name: rule.name.clone(),
            severity: rule.severity,
            from_matcher: rule
                .from
                .as_ref()
                .map(CompiledFilter::compile)
                .transpose()?,
            to_matcher: rule.to.as_ref().map(CompiledFilter::compile).transpose()?,
        })
    }

    const fn severity(&self) -> Option<FindingSeverity> {
        match self.severity {
            AnalysisSeverity::Off => None,
            AnalysisSeverity::Info => Some(FindingSeverity::Info),
            AnalysisSeverity::Warn => Some(FindingSeverity::Warn),
            AnalysisSeverity::Error => Some(FindingSeverity::Error),
        }
    }

    fn matches_from(&self, path: &str, workspace: Option<&str>, package: Option<&str>) -> bool {
        self.from_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.matches(path, workspace, package))
    }

    fn matches_to(&self, path: Option<&str>, workspace: Option<&str>, package: Option<&str>) -> bool {
        self.to_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.matches_optional(path, workspace, package))
    }
}

impl CompiledFilter {
    fn compile(filter: &RuleFilter) -> miette::Result<Self> {
        Ok(Self {
            path: compile_globset(&filter.path)?,
            path_not: compile_globset(&filter.path_not)?,
            workspace: compile_globset(&filter.workspace)?,
            workspace_not: compile_globset(&filter.workspace_not)?,
            package: compile_globset(&filter.package)?,
            package_not: compile_globset(&filter.package_not)?,
        })
    }

    fn matches(&self, path: &str, workspace: Option<&str>, package: Option<&str>) -> bool {
        self.matches_optional(Some(path), workspace, package)
    }

    fn matches_optional(&self, path: Option<&str>, workspace: Option<&str>, package: Option<&str>) -> bool {
        if let Some(matcher) = &self.path
            && !path.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.path_not
            && path.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.workspace
            && !workspace.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.workspace_not
            && workspace.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.package
            && !package.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.package_not
            && package.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        true
    }
}

fn compile_globset(patterns: &[String]) -> miette::Result<Option<GlobSet>> {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;

    for pattern in patterns {
        let glob = Glob::new(pattern)
            .map_err(|err| miette::miette!("invalid glob `{pattern}`: {err}"))?;
        builder.add(glob);
        has_patterns = true;
    }

    if !has_patterns {
        return Ok(None);
    }

    builder
        .build()
        .map(Some)
        .map_err(|err| miette::miette!("failed to compile rule globs: {err}"))
}

fn rule_finding_id(rule_name: &str, from: &str, to: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rule_name.hash(&mut hasher);
    from.hash(&mut hasher);
    to.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
