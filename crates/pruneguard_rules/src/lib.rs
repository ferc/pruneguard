use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use globset::{Glob, GlobSet, GlobSetBuilder};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use pruneguard_config::{
    AnalysisSeverity, OverrideConfig, OwnershipConfig, PruneguardConfig, Rule, RuleFilter,
    RulesConfig, TeamConfig,
};
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_graph::{FileId, GraphBuildResult, ModuleNode, PackageId};
use pruneguard_report::{
    Evidence, Finding, FindingCategory, FindingConfidence, FindingSeverity, RemediationActionKind,
};
use pruneguard_resolver::{ResolutionOutcome, ResolvedEdge, ResolvedEdgeKind};
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
    tag: Option<GlobSet>,
    tag_not: Option<GlobSet>,
    dependency_kinds: Option<GlobSet>,
    profiles: Option<GlobSet>,
    reachable_from: Option<ReachQuery>,
    reaches: Option<ReachQuery>,
    entrypoint_kinds: Option<GlobSet>,
}

#[derive(Debug, Clone)]
struct ReachQuery {
    key: String,
    matcher: GlobSet,
}

#[derive(Debug, Default)]
struct FileContext {
    production: bool,
    development: bool,
    entrypoint_kinds: FxHashSet<String>,
    tags: FxHashSet<String>,
}

#[derive(Default)]
struct RuleContexts {
    files: FxHashMap<FileId, FileContext>,
    package_tags: FxHashMap<String, FxHashSet<String>>,
    workspace_tags: FxHashMap<String, FxHashSet<String>>,
}

struct RuleRuntime<'a> {
    build: &'a GraphBuildResult,
    profile: EntrypointProfile,
    contexts: RuleContexts,
    forward_cache: RefCell<FxHashMap<NodeIndex, FxHashSet<NodeIndex>>>,
    reach_seed_cache: RefCell<FxHashMap<String, FxHashSet<NodeIndex>>>,
}

struct Candidate<'a> {
    path: Option<&'a str>,
    workspace: Option<&'a str>,
    package: Option<&'a str>,
    tags: Option<&'a FxHashSet<String>>,
    context: Option<&'a FileContext>,
    node: Option<NodeIndex>,
}

struct EdgeTarget {
    path: Option<String>,
    workspace: Option<String>,
    package: Option<String>,
    subject: String,
    node: Option<NodeIndex>,
}

struct TeamTagMatcher {
    path_matcher: Option<GlobSet>,
    packages: FxHashSet<String>,
    tags: Vec<String>,
}

struct OverrideTagMatcher {
    file_matcher: Option<GlobSet>,
    workspace_matcher: Option<GlobSet>,
    tags: Vec<String>,
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
            allow: config.allow.iter().map(CompiledRule::compile).collect::<miette::Result<_>>()?,
        })
    }

    /// Evaluate forbidden rules against resolved dependency edges.
    pub fn evaluate(
        &self,
        build: &GraphBuildResult,
        config: &PruneguardConfig,
        profile: EntrypointProfile,
    ) -> Vec<Finding> {
        let runtime = RuleRuntime::new(build, config, profile);
        let mut findings = Vec::new();

        for extracted_file in &build.files {
            let from_path = extracted_file.file.relative_path.to_string_lossy().to_string();
            let Some(file_id) =
                build.module_graph.file_id(&extracted_file.file.path.to_string_lossy())
            else {
                continue;
            };
            let Some(from_node) = build.module_graph.file_index.get(&file_id).copied() else {
                continue;
            };
            let Some(from_context) = runtime.contexts.files.get(&file_id) else {
                continue;
            };
            let from_candidate = Candidate {
                path: Some(&from_path),
                workspace: extracted_file.file.workspace.as_deref(),
                package: extracted_file.file.package.as_deref(),
                tags: Some(&from_context.tags),
                context: Some(from_context),
                node: Some(from_node),
            };

            for edge in
                extracted_file.resolved_imports.iter().chain(&extracted_file.resolved_reexports)
            {
                if matches!(edge.outcome, ResolutionOutcome::Unresolved) {
                    continue;
                }

                let edge_labels = edge_kind_labels(edge);
                let target = edge_target(build, edge);
                let target_tags = runtime
                    .collect_target_tags(target.workspace.as_deref(), target.package.as_deref());
                let target_candidate = Candidate {
                    path: target.path.as_deref(),
                    workspace: target.workspace.as_deref(),
                    package: target.package.as_deref(),
                    tags: (!target_tags.is_empty()).then_some(&target_tags),
                    context: target.node.and_then(|node| match &build.module_graph.graph[node] {
                        ModuleNode::File { id, .. } => runtime.contexts.files.get(id),
                        _ => None,
                    }),
                    node: target.node,
                };

                for rule in &self.forbidden {
                    let Some(severity) = rule.severity() else {
                        continue;
                    };

                    if !rule.matches_from(&runtime, &from_candidate, &edge_labels) {
                        continue;
                    }

                    if !rule.matches_to(&runtime, &target_candidate, &edge_labels) {
                        continue;
                    }

                    findings.push(Finding {
                        id: rule_finding_id(&rule.name, &from_path, &target.subject),
                        code: "boundary-violation".to_string(),
                        severity,
                        category: FindingCategory::BoundaryViolation,
                        confidence: FindingConfidence::High,
                        subject: format!("{from_path} -> {}", target.subject),
                        workspace: extracted_file.file.workspace.clone(),
                        package: extracted_file.file.package.clone(),
                        message: format!(
                            "Rule `{}` forbids `{from_path}` from depending on `{}`.",
                            rule.name, target.subject
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
                            "Adjust the dependency direction or narrow the rule scope.".to_string(),
                        ),
                        rule_name: Some(rule.name.clone()),
                        primary_action_kind: Some(RemediationActionKind::UpdateBoundaryRule),
                        action_kinds: vec![RemediationActionKind::UpdateBoundaryRule],
                        trust_notes: None,
                        framework_context: None,
                        precision_source: None,
                        confidence_reason: None,
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
            from_matcher: rule.from.as_ref().map(CompiledFilter::compile).transpose()?,
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
        runtime: &RuleRuntime<'_>,
        candidate: &Candidate<'_>,
        edge_labels: &[&str],
    ) -> bool {
        self.from_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.matches(runtime, candidate, edge_labels))
    }

    fn matches_to(
        &self,
        runtime: &RuleRuntime<'_>,
        candidate: &Candidate<'_>,
        edge_labels: &[&str],
    ) -> bool {
        self.to_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.matches(runtime, candidate, edge_labels))
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

        if parts.is_empty() { "no additional dimensions".to_string() } else { parts.join("; ") }
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
            tag: compile_globset(&filter.tag)?,
            tag_not: compile_globset(&filter.tag_not)?,
            dependency_kinds: compile_globset(&filter.dependency_kinds)?,
            profiles: compile_globset(&filter.profiles)?,
            reachable_from: ReachQuery::compile(filter.reachable_from.as_ref())?,
            reaches: ReachQuery::compile(filter.reaches.as_ref())?,
            entrypoint_kinds: compile_globset(&filter.entrypoint_kinds)?,
        })
    }

    fn matches(
        &self,
        runtime: &RuleRuntime<'_>,
        candidate: &Candidate<'_>,
        edge_labels: &[&str],
    ) -> bool {
        if let Some(matcher) = &self.path
            && !candidate.path.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.path_not
            && candidate.path.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.workspace
            && !candidate.workspace.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.workspace_not
            && candidate.workspace.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.package
            && !candidate.package.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.package_not
            && candidate.package.is_some_and(|value| matcher.is_match(value))
        {
            return false;
        }

        if let Some(matcher) = &self.tag {
            let Some(tags) = candidate.tags else {
                return false;
            };
            if !tags.iter().any(|tag| matcher.is_match(tag)) {
                return false;
            }
        }

        if let Some(matcher) = &self.tag_not
            && candidate.tags.is_some_and(|tags| tags.iter().any(|tag| matcher.is_match(tag)))
        {
            return false;
        }

        if let Some(matcher) = &self.dependency_kinds
            && !edge_labels.iter().any(|label| matcher.is_match(label))
        {
            return false;
        }

        if let Some(matcher) = &self.profiles {
            let Some(context) = candidate.context else {
                return false;
            };
            if !profile_labels(context).iter().any(|label| matcher.is_match(label)) {
                return false;
            }
        }

        if let Some(matcher) = &self.entrypoint_kinds {
            let Some(context) = candidate.context else {
                return false;
            };
            if !context.entrypoint_kinds.iter().any(|kind| matcher.is_match(kind)) {
                return false;
            }
        }

        if let Some(query) = &self.reachable_from {
            let Some(node) = candidate.node else {
                return false;
            };
            if !runtime.is_reachable_from_query(node, query) {
                return false;
            }
        }

        if let Some(query) = &self.reaches {
            let Some(node) = candidate.node else {
                return false;
            };
            if !runtime.does_reach_query(node, query) {
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
        if self.tag.is_some() {
            dimensions.push("tag");
        }
        if self.tag_not.is_some() {
            dimensions.push("tagNot");
        }
        if self.dependency_kinds.is_some() {
            dimensions.push("dependencyKinds");
        }
        if self.profiles.is_some() {
            dimensions.push("profiles");
        }
        if self.reachable_from.is_some() {
            dimensions.push("reachableFrom");
        }
        if self.reaches.is_some() {
            dimensions.push("reaches");
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

impl ReachQuery {
    fn compile(values: Option<&Vec<String>>) -> miette::Result<Option<Self>> {
        let Some(values) = values else {
            return Ok(None);
        };
        if values.is_empty() {
            return Ok(None);
        }

        let matcher = compile_globset(values)?
            .ok_or_else(|| miette::miette!("reachability query requires at least one pattern"))?;
        let mut patterns = values.clone();
        patterns.sort();
        Ok(Some(Self { key: patterns.join("\u{1f}"), matcher }))
    }
}

impl<'a> RuleRuntime<'a> {
    fn new(
        build: &'a GraphBuildResult,
        config: &PruneguardConfig,
        profile: EntrypointProfile,
    ) -> Self {
        Self {
            build,
            profile,
            contexts: build_rule_contexts(build, config),
            forward_cache: RefCell::new(FxHashMap::default()),
            reach_seed_cache: RefCell::new(FxHashMap::default()),
        }
    }

    fn collect_target_tags(
        &self,
        workspace: Option<&str>,
        package: Option<&str>,
    ) -> FxHashSet<String> {
        let mut tags = FxHashSet::default();
        if let Some(package) = package
            && let Some(package_tags) = self.contexts.package_tags.get(package)
        {
            tags.extend(package_tags.iter().cloned());
        }
        if let Some(workspace) = workspace
            && let Some(workspace_tags) = self.contexts.workspace_tags.get(workspace)
        {
            tags.extend(workspace_tags.iter().cloned());
        }
        tags
    }

    fn is_reachable_from_query(&self, candidate: NodeIndex, query: &ReachQuery) -> bool {
        self.seed_nodes_for_query(query).iter().any(|seed| {
            *seed == candidate || self.forward_reachable_from(*seed).contains(&candidate)
        })
    }

    fn does_reach_query(&self, candidate: NodeIndex, query: &ReachQuery) -> bool {
        let reachable = self.forward_reachable_from(candidate);
        self.seed_nodes_for_query(query)
            .iter()
            .any(|seed| *seed == candidate || reachable.contains(seed))
    }

    fn seed_nodes_for_query(&self, query: &ReachQuery) -> FxHashSet<NodeIndex> {
        if let Some(cached) = self.reach_seed_cache.borrow().get(&query.key).cloned() {
            return cached;
        }

        let mut nodes = FxHashSet::default();
        for index in self.build.module_graph.graph.node_indices() {
            match &self.build.module_graph.graph[index] {
                ModuleNode::File { path, relative_path, workspace, package, .. } => {
                    if query.matcher.is_match(relative_path)
                        || query.matcher.is_match(path)
                        || workspace.as_deref().is_some_and(|value| query.matcher.is_match(value))
                        || package.as_deref().is_some_and(|value| query.matcher.is_match(value))
                    {
                        nodes.insert(index);
                    }
                }
                ModuleNode::Package { name, workspace, path, .. } => {
                    if query.matcher.is_match(name)
                        || query.matcher.is_match(path)
                        || workspace.as_deref().is_some_and(|value| query.matcher.is_match(value))
                    {
                        nodes.insert(index);
                    }
                }
                ModuleNode::Workspace { name, path, .. } => {
                    if query.matcher.is_match(name) || query.matcher.is_match(path) {
                        nodes.insert(index);
                    }
                }
                ModuleNode::Entrypoint { .. } | ModuleNode::ExternalDependency { .. } => {}
            }
        }

        self.reach_seed_cache.borrow_mut().insert(query.key.clone(), nodes.clone());
        nodes
    }

    fn forward_reachable_from(&self, start: NodeIndex) -> FxHashSet<NodeIndex> {
        if let Some(cached) = self.forward_cache.borrow().get(&start).cloned() {
            return cached;
        }

        let mut visited = FxHashSet::default();
        let mut queue = VecDeque::from([start]);

        while let Some(node) = queue.pop_front() {
            if !self.profile_allows_node(node) || !visited.insert(node) {
                continue;
            }

            for edge in self.build.module_graph.graph.edges(node) {
                let target = edge.target();
                if self.profile_allows_node(target) {
                    queue.push_back(target);
                }
            }
        }

        self.forward_cache.borrow_mut().insert(start, visited.clone());
        visited
    }

    fn profile_allows_node(&self, node: NodeIndex) -> bool {
        match &self.build.module_graph.graph[node] {
            ModuleNode::Entrypoint { profile, .. } => match self.profile {
                EntrypointProfile::Both => true,
                EntrypointProfile::Production => {
                    *profile == EntrypointProfile::Production || *profile == EntrypointProfile::Both
                }
                EntrypointProfile::Development => {
                    *profile == EntrypointProfile::Development
                        || *profile == EntrypointProfile::Both
                }
            },
            _ => true,
        }
    }
}

#[allow(clippy::too_many_lines)]
fn build_rule_contexts(build: &GraphBuildResult, config: &PruneguardConfig) -> RuleContexts {
    let reachable_prod = build.module_graph.reachable_file_ids(EntrypointProfile::Production);
    let reachable_dev = build.module_graph.reachable_file_ids(EntrypointProfile::Development);
    let mut files = FxHashMap::default();
    let mut package_tags = package_tags_from_ownership(config.ownership.as_ref());
    let mut workspace_tags = workspace_tags_from_overrides(build, &config.overrides);

    for file_id in build.module_graph.file_index.keys().copied() {
        files.insert(
            file_id,
            FileContext {
                production: reachable_prod.contains(&file_id),
                development: reachable_dev.contains(&file_id),
                entrypoint_kinds: FxHashSet::default(),
                tags: FxHashSet::default(),
            },
        );
    }

    for entrypoint in build.module_graph.entrypoint_nodes(EntrypointProfile::Both) {
        let ModuleNode::Entrypoint { kind, profile, .. } = &build.module_graph.graph[entrypoint]
        else {
            continue;
        };
        let entrypoint_tag = format!("entrypoint-kind:{}", kind.as_str());
        let mut queue = VecDeque::from([entrypoint]);
        let mut visited = FxHashSet::default();

        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }

            for edge in build.module_graph.graph.edges(node) {
                queue.push_back(edge.target());
            }

            if let Some(file_id) = build.module_graph.entrypoint_file_id(node)
                && let Some(context) = files.get_mut(&file_id)
            {
                context.entrypoint_kinds.insert(kind.as_str().to_string());
                context.tags.insert(entrypoint_tag.clone());
                mark_profile_reachability(context, *profile);
            }

            if let ModuleNode::File { id, workspace, package, .. } = &build.module_graph.graph[node]
                && let Some(context) = files.get_mut(id)
            {
                context.entrypoint_kinds.insert(kind.as_str().to_string());
                context.tags.insert(entrypoint_tag.clone());
                mark_profile_reachability(context, *profile);

                if let Some(workspace) = workspace
                    && let Some(tags) = workspace_tags.get(workspace)
                {
                    context.tags.extend(tags.iter().cloned());
                }
                if let Some(package) = package
                    && let Some(tags) = package_tags.get(package)
                {
                    context.tags.extend(tags.iter().cloned());
                }
            }
        }
    }

    let team_matchers = compile_team_tag_matchers(config.ownership.as_ref());
    let override_matchers = compile_override_tag_matchers(&config.overrides);
    for extracted_file in &build.files {
        let Some(file_id) = build.module_graph.file_id(&extracted_file.file.path.to_string_lossy())
        else {
            continue;
        };
        let Some(context) = files.get_mut(&file_id) else {
            continue;
        };
        let relative_path = extracted_file.file.relative_path.to_string_lossy();
        let workspace = extracted_file.file.workspace.as_deref();
        let package = extracted_file.file.package.as_deref();

        if let Some(workspace) = workspace
            && let Some(tags) = workspace_tags.get(workspace)
        {
            context.tags.extend(tags.iter().cloned());
        }
        if let Some(package) = package
            && let Some(tags) = package_tags.get(package)
        {
            context.tags.extend(tags.iter().cloned());
        }

        for matcher in &team_matchers {
            if matcher
                .path_matcher
                .as_ref()
                .is_some_and(|glob| glob.is_match(relative_path.as_ref()))
                || package.is_some_and(|name| matcher.packages.contains(name))
            {
                context.tags.extend(matcher.tags.iter().cloned());
                if let Some(package) = package {
                    package_tags
                        .entry(package.to_string())
                        .or_default()
                        .extend(matcher.tags.iter().cloned());
                }
            }
        }

        for matcher in &override_matchers {
            if matcher.matches(relative_path.as_ref(), workspace) {
                context.tags.extend(matcher.tags.iter().cloned());
                if let Some(workspace) = workspace {
                    workspace_tags
                        .entry(workspace.to_string())
                        .or_default()
                        .extend(matcher.tags.iter().cloned());
                }
            }
        }
    }

    RuleContexts { files, package_tags, workspace_tags }
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

fn edge_target(build: &GraphBuildResult, edge: &ResolvedEdge) -> EdgeTarget {
    if let Some(target_path) = &edge.to_file
        && let Some(target_file) = build.find_file(target_path.to_string_lossy().as_ref())
    {
        return EdgeTarget {
            path: Some(target_file.file.relative_path.to_string_lossy().to_string()),
            workspace: target_file.file.workspace.clone(),
            package: target_file.file.package.clone(),
            subject: target_file.file.relative_path.to_string_lossy().to_string(),
            node: build.module_graph.file_node(&target_file.file.path.to_string_lossy()),
        };
    }

    let dependency = edge.to_dependency.clone().unwrap_or_else(|| edge.specifier.clone());
    if let Some((workspace_name, workspace)) =
        build.discovery.workspaces.iter().find(|(name, workspace)| {
            *name == &dependency || workspace.manifest.name.as_deref() == Some(dependency.as_str())
        })
    {
        let package_name =
            workspace.manifest.name.clone().unwrap_or_else(|| workspace_name.clone());
        return EdgeTarget {
            path: None,
            workspace: Some(workspace_name.clone()),
            package: Some(package_name.clone()),
            subject: dependency,
            node: package_node(build, &package_name),
        };
    }

    EdgeTarget {
        path: None,
        workspace: None,
        package: Some(dependency.clone()),
        subject: dependency,
        node: None,
    }
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
        ResolvedEdgeKind::RequireResolve => vec!["require-resolve"],
        ResolvedEdgeKind::ImportMetaGlob => vec!["import-meta-glob"],
        ResolvedEdgeKind::JsDocImport => vec!["jsdoc-import", "type"],
        ResolvedEdgeKind::TripleSlashFile => vec!["triple-slash", "triple-slash-file"],
        ResolvedEdgeKind::TripleSlashTypes => vec!["triple-slash", "triple-slash-types", "type"],
        ResolvedEdgeKind::ImportMetaResolve => vec!["import-meta-resolve"],
        ResolvedEdgeKind::RequireContext => vec!["require-context"],
        ResolvedEdgeKind::UrlConstructor => vec!["url-constructor"],
        ResolvedEdgeKind::ImportEquals => vec!["import-equals", "require"],
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

fn package_tags_from_ownership(
    ownership: Option<&OwnershipConfig>,
) -> FxHashMap<String, FxHashSet<String>> {
    let mut tags = FxHashMap::<String, FxHashSet<String>>::default();
    let Some(teams) = ownership.and_then(|config| config.teams.as_ref()) else {
        return tags;
    };

    for TeamConfig { packages, tags: team_tags, .. } in teams.values() {
        if team_tags.is_empty() {
            continue;
        }
        for package in packages {
            tags.entry(package.clone()).or_default().extend(team_tags.iter().cloned());
        }
    }

    tags
}

fn workspace_tags_from_overrides(
    build: &GraphBuildResult,
    overrides: &[OverrideConfig],
) -> FxHashMap<String, FxHashSet<String>> {
    let matchers = compile_override_tag_matchers(overrides);
    let mut tags = FxHashMap::<String, FxHashSet<String>>::default();

    for workspace_name in build.discovery.workspaces.keys() {
        for matcher in &matchers {
            if matcher.matches_workspace_only(workspace_name) {
                tags.entry(workspace_name.clone())
                    .or_default()
                    .extend(matcher.tags.iter().cloned());
            }
        }
    }

    tags
}

fn compile_team_tag_matchers(ownership: Option<&OwnershipConfig>) -> Vec<TeamTagMatcher> {
    let Some(teams) = ownership.and_then(|config| config.teams.as_ref()) else {
        return Vec::new();
    };

    let mut matchers = teams
        .iter()
        .filter(|(_, config)| !config.tags.is_empty())
        .map(|(_, config)| TeamTagMatcher {
            path_matcher: compile_globset(&config.paths).ok().flatten(),
            packages: config.packages.iter().cloned().collect(),
            tags: config.tags.clone(),
        })
        .collect::<Vec<_>>();
    matchers.sort_by(|left, right| left.tags.cmp(&right.tags));
    matchers
}

fn compile_override_tag_matchers(overrides: &[OverrideConfig]) -> Vec<OverrideTagMatcher> {
    overrides
        .iter()
        .filter(|override_config| !override_config.tags.is_empty())
        .map(|override_config| OverrideTagMatcher {
            file_matcher: compile_globset(&override_config.files).ok().flatten(),
            workspace_matcher: compile_globset(&override_config.workspaces).ok().flatten(),
            tags: override_config.tags.clone(),
        })
        .collect()
}

impl OverrideTagMatcher {
    fn matches(&self, relative_path: &str, workspace: Option<&str>) -> bool {
        let file_ok =
            self.file_matcher.as_ref().is_none_or(|matcher| matcher.is_match(relative_path));
        let workspace_ok = self
            .workspace_matcher
            .as_ref()
            .is_none_or(|matcher| workspace.is_some_and(|value| matcher.is_match(value)));
        file_ok && workspace_ok
    }

    fn matches_workspace_only(&self, workspace: &str) -> bool {
        self.file_matcher.is_none()
            && self.workspace_matcher.as_ref().is_some_and(|matcher| matcher.is_match(workspace))
    }
}

fn package_node(build: &GraphBuildResult, package_name: &str) -> Option<NodeIndex> {
    let raw = build.module_graph.interner.lookup(package_name)?;
    build.module_graph.package_index.get(&PackageId(raw)).copied()
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
