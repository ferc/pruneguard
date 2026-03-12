use std::path::{Path, PathBuf};

use oxgraph_config::OxgraphConfig;
use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_graph::{GraphBuildResult, ModuleNode, build_graph};
use oxgraph_report::{AnalysisReport, ExplainReport, Finding, ImpactReport, ProofNode, Summary};

#[cfg(feature = "napi")]
use napi_derive::napi;

/// Run a full scan and return the analysis report.
pub fn scan(
    cwd: &Path,
    config: &OxgraphConfig,
    paths: &[PathBuf],
    profile: EntrypointProfile,
) -> miette::Result<AnalysisReport> {
    let build = build_graph(cwd, config, paths, profile)?;
    let findings = oxgraph_analyzers::run_analyzers(&build, config, profile);
    Ok(report_from_build(cwd, &build, findings, profile))
}

/// Compute the blast radius for a target.
pub fn impact(
    cwd: &Path,
    config: &OxgraphConfig,
    target: &str,
    profile: EntrypointProfile,
) -> miette::Result<ImpactReport> {
    let build = build_graph(cwd, config, &[], profile)?;
    let target_file = build
        .find_file(target)
        .ok_or_else(|| miette::miette!("target `{target}` did not match a tracked file"))?;
    let file_id = build
        .module_graph
        .file_id(&target_file.file.path.to_string_lossy())
        .ok_or_else(|| miette::miette!("target `{target}` is not present in the module graph"))?;

    let analysis = oxgraph_analyzers::impact::analyze(&build, file_id, profile);
    let proofs = build
        .module_graph
        .shortest_path_to_file(file_id, profile)
        .map(|path| {
            path.iter()
                .map(|index| node_label(&build, *index))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ImpactReport {
        target: target_file.file.relative_path.to_string_lossy().to_string(),
        affected_entrypoints: analysis.affected_entrypoints,
        affected_packages: analysis.affected_packages,
        affected_files: analysis.affected_files,
        evidence: if proofs.is_empty() {
            Vec::new()
        } else {
            vec![oxgraph_report::Evidence {
                kind: "path".to_string(),
                file: Some(target_file.file.relative_path.to_string_lossy().to_string()),
                line: None,
                description: format!("One entrypoint path to the target: {}", proofs.join(" -> ")),
            }]
        },
    })
}

/// Explain a finding or path.
pub fn explain(
    cwd: &Path,
    config: &OxgraphConfig,
    query: &str,
    profile: EntrypointProfile,
) -> miette::Result<ExplainReport> {
    let build = build_graph(cwd, config, &[], profile)?;
    let findings = oxgraph_analyzers::run_analyzers(&build, config, profile);
    let related_findings = findings
        .iter()
        .filter(|finding| finding.id == query || finding.subject == query)
        .cloned()
        .collect::<Vec<_>>();

    let target_query = related_findings
        .first()
        .map_or(query, |finding| finding.subject.as_str());
    let matched_file = build.find_file(target_query).or_else(|| {
        target_query
            .split_once('#')
            .and_then(|(path, _)| build.find_file(path))
    });

    let matched_node = matched_file
        .map(|file| file.file.relative_path.to_string_lossy().to_string())
        .or_else(|| related_findings.first().map(|finding| finding.subject.clone()));

    let proofs = matched_file
        .and_then(|file| {
                build.module_graph.file_id(&file.file.path.to_string_lossy()).and_then(|file_id| {
                    build.module_graph.shortest_path_to_file(file_id, profile).map(|path| {
                        build_proof_tree(
                            path.iter().map(|index| node_label(&build, *index)).collect::<Vec<_>>(),
                        )
                    })
                })
            })
        .into_iter()
        .collect::<Vec<_>>();

    Ok(ExplainReport { query: query.to_string(), matched_node, proofs, related_findings })
}

/// Debug: list all detected entrypoints.
pub fn debug_entrypoints(
    cwd: &Path,
    config: &OxgraphConfig,
    profile: EntrypointProfile,
) -> miette::Result<Vec<String>> {
    let build = build_graph(cwd, config, &[], profile)?;
    Ok(build.entrypoint_seeds.iter().map(ToString::to_string).collect())
}

/// Debug: resolve a specifier from a file.
pub fn debug_resolve(cwd: &Path, config: &OxgraphConfig, specifier: &str, from: &Path) -> String {
    oxgraph_resolver::debug_resolve(cwd, &config.resolver, specifier, from)
}

fn report_from_build(
    cwd: &Path,
    build: &GraphBuildResult,
    findings: Vec<Finding>,
    profile: EntrypointProfile,
) -> AnalysisReport {
    let (errors, warnings, infos) = finding_summary(&findings);

    AnalysisReport {
        version: 1,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        cwd: cwd.to_string_lossy().to_string(),
        profile: profile.as_str().to_string(),
        summary: Summary {
            total_files: build.inventories.files.len(),
            total_packages: build.inventories.packages.len(),
            total_workspaces: build.inventories.workspaces.len(),
            total_exports: build.symbol_graph.exports.len(),
            total_findings: findings.len(),
            errors,
            warnings,
            infos,
        },
        inventories: build.inventories.clone(),
        findings,
        entrypoints: build.entrypoints.clone(),
        stats: build.stats.clone(),
    }
}

fn build_proof_tree(nodes: Vec<String>) -> ProofNode {
    if nodes.is_empty() {
        return ProofNode {
            node: "unreachable".to_string(),
            relationship: "root".to_string(),
            children: Vec::new(),
        };
    }

    let mut iter = nodes.into_iter().rev();
    let mut current = ProofNode {
        node: iter.next().unwrap_or_default(),
        relationship: "target".to_string(),
        children: Vec::new(),
    };

    for node in iter {
        current = ProofNode {
            node,
            relationship: "depends-on".to_string(),
            children: vec![current],
        };
    }

    current
}

fn finding_summary(findings: &[Finding]) -> (usize, usize, usize) {
    findings.iter().fold((0, 0, 0), |(errors, warnings, infos), finding| {
        match finding.severity {
            oxgraph_report::FindingSeverity::Error => (errors + 1, warnings, infos),
            oxgraph_report::FindingSeverity::Warn => (errors, warnings + 1, infos),
            oxgraph_report::FindingSeverity::Info => (errors, warnings, infos + 1),
        }
    })
}

fn node_label(build: &GraphBuildResult, index: petgraph::graph::NodeIndex) -> String {
    match &build.module_graph.graph[index] {
        ModuleNode::Entrypoint { path, .. } => {
            let path = Path::new(path);
            path.strip_prefix(&build.discovery.project_root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string()
        }
        ModuleNode::File { relative_path, .. } => relative_path.clone(),
        ModuleNode::Package { name, .. }
        | ModuleNode::Workspace { name, .. }
        | ModuleNode::ExternalDependency { name } => name.clone(),
    }
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsScanOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub paths: Option<Vec<String>>,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsImpactOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub target: String,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsExplainOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub query: String,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsDebugResolveOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub specifier: String,
    pub from: String,
}

#[cfg(feature = "napi")]
#[napi(object)]
pub struct JsDebugEntrypointsOptions {
    pub cwd: Option<String>,
    pub config: Option<String>,
    pub profile: Option<String>,
}

#[cfg(feature = "napi")]
fn load_config_for_js(
    cwd: &Path,
    config_path: Option<&str>,
) -> napi::Result<OxgraphConfig> {
    match OxgraphConfig::load(cwd, config_path.map(Path::new)) {
        Ok(config) => Ok(config),
        Err(oxgraph_config::ConfigError::NotFound) => Ok(OxgraphConfig::default()),
        Err(err) => Err(napi::Error::from_reason(err.to_string())),
    }
}

#[cfg(feature = "napi")]
fn parse_profile(profile: Option<&str>) -> EntrypointProfile {
    match profile {
        Some("production") => EntrypointProfile::Production,
        Some("development") => EntrypointProfile::Development,
        _ => EntrypointProfile::Both,
    }
}

#[cfg(feature = "napi")]
#[napi]
pub fn scan_json(options: JsScanOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let report = scan(
        &cwd,
        &config,
        &options
            .paths
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>(),
        parse_profile(options.profile.as_deref()),
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn impact_json(options: JsImpactOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let report = impact(
        &cwd,
        &config,
        &options.target,
        parse_profile(options.profile.as_deref()),
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn explain_json(options: JsExplainOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let report = explain(
        &cwd,
        &config,
        &options.query,
        parse_profile(options.profile.as_deref()),
    )
    .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&report).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn load_config_json(cwd: Option<String>, config: Option<String>) -> napi::Result<String> {
    let cwd = cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, config.as_deref())?;
    serde_json::to_string(&config).map_err(|err| napi::Error::from_reason(err.to_string()))
}

#[cfg(feature = "napi")]
#[napi]
pub fn debug_resolve_text(options: JsDebugResolveOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    Ok(debug_resolve(&cwd, &config, &options.specifier, Path::new(&options.from)))
}

#[cfg(feature = "napi")]
#[napi]
pub fn debug_entrypoints_json(options: JsDebugEntrypointsOptions) -> napi::Result<String> {
    let cwd = options
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let config = load_config_for_js(&cwd, options.config.as_deref())?;
    let entrypoints = debug_entrypoints(&cwd, &config, parse_profile(options.profile.as_deref()))
        .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    serde_json::to_string(&entrypoints).map_err(|err| napi::Error::from_reason(err.to_string()))
}
