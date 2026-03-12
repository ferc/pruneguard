use std::path::{Path, PathBuf};
use std::process::Command;

use miette::IntoDiagnostic;
use oxgraph_config::{
    AnalysisSeverity, EntrypointsConfig, OxgraphConfig, ResolverConfig, Rule, RuleFilter,
    RulesConfig, WorkspacesConfig,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationOutput {
    pub source: String,
    pub config: OxgraphConfig,
    pub warnings: Vec<String>,
}

pub fn migrate_knip(cwd: &Path, file: Option<&Path>) -> miette::Result<MigrationOutput> {
    let (source, value) = load_knip_config(cwd, file)?;
    let mut warnings = Vec::new();
    let mut config = OxgraphConfig {
        schema: Some("./node_modules/oxgraph/configuration_schema.json".to_string()),
        ..OxgraphConfig::default()
    };

    if let Some(workspaces) = value.get("workspaces") {
        let roots = collect_string_values(workspaces);
        if roots.is_empty() {
            warnings.push("knip `workspaces` could not be mapped precisely; review manually.".to_string());
        } else {
            config.workspaces = Some(WorkspacesConfig {
                roots,
                ..WorkspacesConfig::default()
            });
        }
    }

    if let Some(entry) = value.get("entry") {
        config.entrypoints = EntrypointsConfig {
            include: collect_string_values(entry),
            ..config.entrypoints
        };
    }

    if let Some(project) = value.get("project") {
        let includes = collect_string_values(project);
        if !includes.is_empty() {
            let workspaces = config.workspaces.get_or_insert_with(WorkspacesConfig::default);
            workspaces.include.extend(includes);
            workspaces.include.sort();
            workspaces.include.dedup();
        }
    }

    for (key, value) in value.as_object().into_iter().flat_map(|object| object.iter()) {
        if !key.starts_with("ignore") {
            continue;
        }
        let ignores = collect_string_values(value);
        if ignores.is_empty() {
            warnings.push(format!("knip `{key}` was present but could not be mapped."));
            continue;
        }
        config.ignore_patterns.extend(ignores);
    }

    config.ignore_patterns.sort();
    config.ignore_patterns.dedup();

    Ok(MigrationOutput { source, config, warnings })
}

pub fn migrate_depcruise(
    cwd: &Path,
    file: Option<&Path>,
    node: bool,
) -> miette::Result<MigrationOutput> {
    let path = resolve_depcruise_path(cwd, file)?;
    let value = load_depcruise_config(&path, node)?;
    let mut warnings = Vec::new();
    let mut config = OxgraphConfig {
        schema: Some("./node_modules/oxgraph/configuration_schema.json".to_string()),
        ..OxgraphConfig::default()
    };

    if let Some(forbidden) = value.get("forbidden") {
        let rules = forbidden
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(map_depcruise_rule)
            .collect::<Vec<_>>();
        if !rules.is_empty() {
            config.rules.get_or_insert_with(RulesConfig::default).forbidden = rules;
        }
    }

    if let Some(required) = value.get("required") {
        let rules = required
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(map_depcruise_rule)
            .collect::<Vec<_>>();
        if !rules.is_empty() {
            config.rules.get_or_insert_with(RulesConfig::default).required = rules;
        }
    }

    if let Some(options) = value.get("options") {
        if let Some(tsconfig) = options.get("tsConfig") {
            let resolver = ResolverConfig {
                tsconfig: collect_string_values(tsconfig),
                ..config.resolver
            };
            config.resolver = resolver;
        }

        if let Some(include_only) = options.get("includeOnly") {
            let workspaces = config.workspaces.get_or_insert_with(WorkspacesConfig::default);
            workspaces.include.extend(collect_string_values(include_only));
        }

        if let Some(exclude) = options.get("exclude") {
            config.ignore_patterns.extend(collect_string_values(exclude));
        }

        if options.get("reporterOptions").is_some() {
            warnings.push("dependency-cruiser reporterOptions are not migrated.".to_string());
        }
        if options.get("knownViolations").is_some() {
            warnings.push("dependency-cruiser knownViolations are not migrated; use baseline.json.".to_string());
        }
    }

    if value.get("options").is_none()
        && value.get("forbidden").is_none()
        && value.get("required").is_none()
    {
        warnings.push("dependency-cruiser config did not expose recognized top-level fields.".to_string());
    }

    config.ignore_patterns.sort();
    config.ignore_patterns.dedup();

    Ok(MigrationOutput {
        source: path.to_string_lossy().to_string(),
        config,
        warnings,
    })
}

fn load_knip_config(cwd: &Path, file: Option<&Path>) -> miette::Result<(String, Value)> {
    if let Some(file) = file {
        let path = resolve_explicit_path(cwd, file);
        let content = std::fs::read_to_string(&path).into_diagnostic()?;
        let value = serde_json::from_str(&content).into_diagnostic()?;
        return Ok((path.to_string_lossy().to_string(), value));
    }

    let knip_json = cwd.join("knip.json");
    if knip_json.exists() {
        let value = serde_json::from_str(
            &std::fs::read_to_string(&knip_json).into_diagnostic()?,
        )
        .into_diagnostic()?;
        return Ok((knip_json.to_string_lossy().to_string(), value));
    }

    let package_json = cwd.join("package.json");
    if package_json.exists() {
        let value: Value = serde_json::from_str(
            &std::fs::read_to_string(&package_json).into_diagnostic()?,
        )
        .into_diagnostic()?;
        if let Some(knip) = value.get("knip") {
            return Ok(("package.json#knip".to_string(), knip.clone()));
        }
    }

    miette::bail!("could not find knip configuration")
}

fn resolve_depcruise_path(cwd: &Path, file: Option<&Path>) -> miette::Result<PathBuf> {
    if let Some(file) = file {
        return Ok(resolve_explicit_path(cwd, file));
    }

    for candidate in [
        ".dependency-cruiser.json",
        ".dependency-cruiser.js",
        ".dependency-cruiser.cjs",
        ".dependency-cruiser.mjs",
    ] {
        let path = cwd.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }

    miette::bail!("could not find dependency-cruiser configuration")
}

fn load_depcruise_config(path: &Path, node: bool) -> miette::Result<Value> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
    if extension == "json" {
        return serde_json::from_str(&std::fs::read_to_string(path).into_diagnostic()?)
            .into_diagnostic();
    }

    let content = std::fs::read_to_string(path).into_diagnostic()?;
    if let Some(value) = parse_static_module_export(&content) {
        return Ok(value);
    }

    if !node {
        miette::bail!("dynamic dependency-cruiser config requires `--node`");
    }

    let path_arg = path.to_string_lossy().to_string();
    let output = Command::new("node")
        .arg("--input-type=module")
        .arg("-e")
        .arg("import { pathToFileURL } from 'node:url'; const target = process.argv[1]; import(pathToFileURL(target).href).then(m => console.log(JSON.stringify(m.default ?? m))).catch(async () => { const { createRequire } = await import('node:module'); const require = createRequire(import.meta.url); console.log(JSON.stringify(require(target))); });")
        .arg(path_arg)
        .output()
        .into_diagnostic()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        miette::bail!("failed to evaluate dependency-cruiser config with node: {stderr}");
    }

    let stdout = String::from_utf8(output.stdout).into_diagnostic()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        miette::bail!("dependency-cruiser config evaluation returned empty output");
    }

    serde_json::from_str(trimmed).into_diagnostic()
}

fn parse_static_module_export(content: &str) -> Option<Value> {
    let trimmed = content.trim();
    for prefix in ["export default", "module.exports ="] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let candidate = rest.trim().trim_end_matches(';').trim();
            if let Ok(value) = serde_json::from_str(candidate) {
                return Some(value);
            }
        }
    }
    None
}

fn map_depcruise_rule(value: &Value) -> Option<Rule> {
    let name = value.get("name").and_then(Value::as_str)?.to_string();
    let severity = match value.get("severity").and_then(Value::as_str) {
        Some("error") => AnalysisSeverity::Error,
        Some("info") => AnalysisSeverity::Info,
        Some("warn" | "warning" | _) | None => AnalysisSeverity::Warn,
    };
    let from = value.get("from").map(map_depcruise_filter);
    let to = value.get("to").map(map_depcruise_filter);
    Some(Rule {
        name,
        severity,
        comment: value
            .get("comment")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        from,
        to,
    })
}

fn map_depcruise_filter(value: &Value) -> RuleFilter {
    RuleFilter {
        path: collect_string_values(value.get("path").unwrap_or(&Value::Null)),
        path_not: collect_string_values(value.get("pathNot").unwrap_or(&Value::Null)),
        workspace: collect_string_values(value.get("workspace").unwrap_or(&Value::Null)),
        workspace_not: collect_string_values(value.get("workspaceNot").unwrap_or(&Value::Null)),
        package: collect_string_values(value.get("package").unwrap_or(&Value::Null)),
        package_not: collect_string_values(value.get("packageNot").unwrap_or(&Value::Null)),
        ..RuleFilter::default()
    }
}

fn collect_string_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
        Value::Object(map) => map
            .values()
            .flat_map(collect_string_values)
            .collect(),
        Value::Bool(_) | Value::Null | Value::Number(_) => Vec::new(),
    }
}

fn resolve_explicit_path(cwd: &Path, file: &Path) -> PathBuf {
    if file.is_absolute() {
        file.to_path_buf()
    } else {
        cwd.join(file)
    }
}
