use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use globset::{Glob, GlobSet, GlobSetBuilder};
use oxgraph_config::{AnalysisSeverity, Rule, RuleFilter, RulesConfig};
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_graph::{FileId, GraphBuildResult, ModuleNode};
use oxgraph_report::{Evidence, Finding, FindingCategory, FindingSeverity};
use oxgraph_resolver::{ResolutionOutcome, ResolvedEdge, ResolvedEdgeKind};
use petgraph::visit::EdgeRef;
use rustc_hash::{FxHashMap, FxHashSet};

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
    dependency_kinds: Option<GlobSet>,
    profiles: Option<GlobSet>,
    entrypoint_kinds: Option<GlobSet>,
}

#[derive(Debug, Default)]
struct FileContext {
    production: bool,
    development: bool,
    entrypoint_kinds: FxHashSet<String>,
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

    /// Evaluate forbidden rules against resolved dependency edges.
    pub fn evaluate(&self, build: &GraphBuildResult) -> Vec<Finding> {
        let file_contexts = build_file_contexts(build);
        let mut findings = Vec::new();

        for extracted_file in &build.files {
            let from_path = extracted_file.file.relative_path.to_string_lossy().to_string();
            let Some(from_context) = build
                .module_graph
                .file_id(&extracted_file.file.path.to_string_lossy())
                .and_then(|file_id| file_contexts.get(&file_id))
            else {
                continue;
            };

            for edge in extracted_file
                .resolved_imports
                .iter()
                .chain(&extracted_file.resolved_reexports)
            {
                if matches!(edge.outcome, ResolutionOutcome::Unresolved) {
                    continue;
                }

                let edge_labels = edge_kind_labels(edge);
                let (to_path, to_workspace, to_package, subject, to_context) =
                    edge_target(build, &file_contexts, edge);

                for rule in &self.forbidden {
                    let Some(severity) = rule.severity() else {
                        continue;
                    };

                    if !rule.matches_from(
                        &from_path,
                        extracted_file.file.workspace.as_deref(),
                        extracted_file.file.package.as_deref(),
                        Some(from_context),
                        &edge_labels,
                    ) {
                        continue;
                    }

                    if !rule.matches_to(
                        to_path.as_deref(),
                        to_workspace.as_deref(),
                        to_package.as_deref(),
                        to_context,
                        &edge_labels,
                    ) {
                        continue;
                    }

                    findings.push(Finding {
                        id: rule_finding_id(&rule.name, &from_path, &subject),
                        code: "boundary-violation".to_string(),
                        severity,
                        category: FindingCategory::BoundaryViolation,
                        subject: format!("{from_path} -> {subject}"),
                        workspace: extracted_file.file.workspace.clone(),
                        package: extracted_file.file.package.clone(),
                        message: format!(
                            "Rule `{}` forbids `{from_path}` from depending on `{subject}`.",
                            rule.name
                        ),
                        evidence: vec![Evidence {
                            kind: "rule".to_string(),
                            file: Some(from_path.clone()),
                            line: edge.line.map(|line| line as usize),
                            description: format!(
                                "Matched forbidden rule `{}` on {} with filters: {}.",
                                rule.name,
                                edge_labels.join(", "),
                                rule.describe_matches()
                            ),
                        }],
                        suggestion: Some(
                            "Adjust the dependency direction or narrow the rule scope."
                                .to_string(),
                        ),
                        rule_name: Some(rule.name.clone()),
                    });
                }
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

    fn matches_from(
        &self,
        path: &str,
        workspace: Option<&str>,
        package: Option<&str>,
        context: Option<&FileContext>,
        edge_labels: &[&str],
    ) -> bool {
        self.from_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.matches(path, workspace, package, context, edge_labels))
    }

    fn matches_to(
        &self,
        path: Option<&str>,
        workspace: Option<&str>,
        package: Option<&str>,
        context: Option<&FileContext>,
        edge_labels: &[&str],
    ) -> bool {
        self.to_matcher.as_ref().is_none_or(|matcher| {
            matcher.matches_optional(path, workspace, package, context, edge_labels)
        })
    }

    fn describe_matches(&self) -> String {
        let mut parts = Vec::new();
        if let Some(filter) = &self.from_matcher {
            let description = filter.describe("from");
            if !description.is_empty() {
                parts.push(description);
            }
        }
        if let Some(filter) = &self.to_matcher {
            let description = filter.describe("to");
            if !description.is_empty() {
                parts.push(description);
            }
        }

        if parts.is_empty() {
            "no additional dimensions".to_string()
        } else {
            parts.join("; ")
        }
    }
}

impl CompiledFilter {
    fn compile(filter: &RuleFilter) -> miette::Result<Self> {
        reject_unsupported_filter_fields(filter)?;
        Ok(Self {
            path: compile_globset(&filter.path)?,
            path_not: compile_globset(&filter.path_not)?,
            workspace: compile_globset(&filter.workspace)?,
            workspace_not: compile_globset(&filter.workspace_not)?,
            package: compile_globset(&filter.package)?,
            package_not: compile_globset(&filter.package_not)?,
            dependency_kinds: compile_globset(&filter.dependency_kinds)?,
            profiles: compile_globset(&filter.profiles)?,
            entrypoint_kinds: compile_globset(&filter.entrypoint_kinds)?,
        })
    }

    fn matches(
        &self,
        path: &str,
        workspace: Option<&str>,
        package: Option<&str>,
        context: Option<&FileContext>,
        edge_labels: &[&str],
    ) -> bool {
        self.matches_optional(Some(path), workspace, package, context, edge_labels)
    }

    fn matches_optional(
        &self,
        path: Option<&str>,
        workspace: Option<&str>,
        package: Option<&str>,
        context: Option<&FileContext>,
        edge_labels: &[&str],
    ) -> bool {
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

        if let Some(matcher) = &self.dependency_kinds
            && !edge_labels.iter().any(|label| matcher.is_match(label))
        {
            return false;
        }

        if let Some(matcher) = &self.profiles {
            let Some(context) = context else {
                return false;
            };
            if !profile_labels(context).iter().any(|label| matcher.is_match(label)) {
                return false;
            }
        }

        if let Some(matcher) = &self.entrypoint_kinds {
            let Some(context) = context else {
                return false;
            };
            if !context
                .entrypoint_kinds
                .iter()
                .any(|kind| matcher.is_match(kind))
            {
                return false;
            }
        }

        true
    }

    fn describe(&self, prefix: &str) -> String {
        let mut dimensions = Vec::new();
        if self.path.is_some() {
            dimensions.push("path");
        }
        if self.path_not.is_some() {
            dimensions.push("pathNot");
        }
        if self.workspace.is_some() {
            dimensions.push("workspace");
        }
        if self.workspace_not.is_some() {
            dimensions.push("workspaceNot");
        }
        if self.package.is_some() {
            dimensions.push("package");
        }
        if self.package_not.is_some() {
            dimensions.push("packageNot");
        }
        if self.dependency_kinds.is_some() {
            dimensions.push("dependencyKinds");
        }
        if self.profiles.is_some() {
            dimensions.push("profiles");
        }
        if self.entrypoint_kinds.is_some() {
            dimensions.push("entrypointKinds");
        }

        if dimensions.is_empty() {
            String::new()
        } else {
            format!("{prefix}[{}]", dimensions.join(","))
        }
    }
}

fn build_file_contexts(build: &GraphBuildResult) -> FxHashMap<FileId, FileContext> {
    let reachable_prod = build.module_graph.reachable_file_ids(EntrypointProfile::Production);
    let reachable_dev = build.module_graph.reachable_file_ids(EntrypointProfile::Development);
    let mut contexts = FxHashMap::default();

    for file_id in build.module_graph.file_index.keys().copied() {
        contexts.insert(
            file_id,
            FileContext {
                production: reachable_prod.contains(&file_id),
                development: reachable_dev.contains(&file_id),
                entrypoint_kinds: FxHashSet::default(),
            },
        );
    }

    for entrypoint in build.module_graph.entrypoint_nodes(EntrypointProfile::Both) {
        let ModuleNode::Entrypoint { kind, profile, .. } = &build.module_graph.graph[entrypoint]
        else {
            continue;
        };
        let mut queue = VecDeque::from([entrypoint]);
        let mut visited = FxHashSet::default();

        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }

            for edge in build.module_graph.graph.edges(node) {
                let next = edge.target();
                queue.push_back(next);
            }

            if let Some(file_id) = build.module_graph.entrypoint_file_id(node)
                && let Some(context) = contexts.get_mut(&file_id)
            {
                context.entrypoint_kinds.insert(kind.as_str().to_string());
                mark_profile_reachability(context, *profile);
            }

            if let ModuleNode::File { id, .. } = &build.module_graph.graph[node]
                && let Some(context) = contexts.get_mut(id)
            {
                context.entrypoint_kinds.insert(kind.as_str().to_string());
                mark_profile_reachability(context, *profile);
            }
        }
    }

    contexts
}

const fn mark_profile_reachability(context: &mut FileContext, profile: EntrypointProfile) {
    match profile {
        EntrypointProfile::Production => context.production = true,
        EntrypointProfile::Development => context.development = true,
        EntrypointProfile::Both => {
            context.production = true;
            context.development = true;
        }
    }
}

fn edge_target<'a>(
    build: &GraphBuildResult,
    file_contexts: &'a FxHashMap<FileId, FileContext>,
    edge: &ResolvedEdge,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<&'a FileContext>,
) {
    if let Some(target_path) = &edge.to_file
        && let Some(target_file) = build.find_file(target_path.to_string_lossy().as_ref())
    {
        let relative_path = target_file.file.relative_path.to_string_lossy().to_string();
        let context = build
            .module_graph
            .file_id(&target_file.file.path.to_string_lossy())
            .and_then(|file_id| file_contexts.get(&file_id));
        return (
            Some(relative_path.clone()),
            target_file.file.workspace.clone(),
            target_file.file.package.clone(),
            relative_path,
            context,
        );
    }

    let dependency = edge
        .to_dependency
        .clone()
        .unwrap_or_else(|| edge.specifier.clone());
    if let Some((workspace_name, workspace)) = build
        .discovery
        .workspaces
        .iter()
        .find(|(name, workspace)| {
            *name == &dependency || workspace.manifest.name.as_deref() == Some(dependency.as_str())
        })
    {
        return (
            None,
            Some(workspace_name.clone()),
            workspace.manifest.name.clone().or_else(|| Some(workspace_name.clone())),
            dependency,
            None,
        );
    }

    (None, None, Some(dependency.clone()), dependency, None)
}

fn edge_kind_labels(edge: &ResolvedEdge) -> Vec<&'static str> {
    let mut labels = match edge.kind {
        ResolvedEdgeKind::StaticImportValue => vec!["static", "static-value"],
        ResolvedEdgeKind::StaticImportType => vec!["static", "static-type", "type"],
        ResolvedEdgeKind::DynamicImport => vec!["dynamic"],
        ResolvedEdgeKind::Require => vec!["require"],
        ResolvedEdgeKind::SideEffectImport => vec!["side-effect"],
        ResolvedEdgeKind::ReExportNamed => vec!["re-export", "re-export-named"],
        ResolvedEdgeKind::ReExportAll => vec!["re-export", "re-export-all"],
    };
    if edge.to_dependency.is_some() {
        labels.push("external");
    }
    labels
}

fn profile_labels(context: &FileContext) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if context.production {
        labels.push("production");
    }
    if context.development {
        labels.push("development");
    }
    if context.production || context.development {
        labels.push("all");
    }
    labels
}

fn reject_unsupported_filter_fields(filter: &RuleFilter) -> miette::Result<()> {
    if !filter.tag.is_empty() || !filter.tag_not.is_empty() {
        miette::bail!("rule tags are not implemented yet");
    }
    if filter.reachable_from.as_ref().is_some_and(|values| !values.is_empty()) {
        miette::bail!("rule reachableFrom is not implemented yet");
    }
    if filter.reaches.as_ref().is_some_and(|values| !values.is_empty()) {
        miette::bail!("rule reaches is not implemented yet");
    }
    Ok(())
}

fn compile_globset(patterns: &[String]) -> miette::Result<Option<GlobSet>> {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|err| miette::miette!("{err}"))?);
        has_patterns = true;
    }

    if !has_patterns {
        return Ok(None);
    }

    builder.build().map(Some).map_err(|err| miette::miette!("{err}"))
}

fn rule_finding_id(rule_name: &str, from: &str, to: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rule_name.hash(&mut hasher);
    from.hash(&mut hasher);
    to.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
