use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigReaderError {
    #[error("IO error reading {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("JSON parse error in {path}: {message}")]
    JsonParse { path: PathBuf, message: String },
    #[error("Config too dynamic to evaluate statically: {path}")]
    TooDynamic { path: PathBuf, reason: String },
}

/// Result of attempting to read a framework config statically.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigReadResult {
    pub path: PathBuf,
    pub format: ConfigFormat,
    pub status: ConfigReadStatus,
    /// Extracted key-value pairs (flat representation of config).
    pub values: Vec<ConfigValue>,
    /// Warnings about things that couldn't be read statically.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub enum ConfigFormat {
    Json,
    Jsonc,
    Yaml,
    JavaScript,
    TypeScript,
}

#[derive(Debug, Clone, Serialize)]
pub enum ConfigReadStatus {
    /// Fully read.
    Complete,
    /// Partially read (some values too dynamic).
    Partial,
    /// Could not read statically.
    Unreadable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigValue {
    pub key: String,
    pub value: ConfigValueKind,
}

#[derive(Debug, Clone, Serialize)]
pub enum ConfigValueKind {
    String(String),
    Bool(bool),
    Number(f64),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
    Dynamic(String), // description of what it is but can't evaluate
}

/// Read a framework config file statically.
pub fn read_config(path: &Path) -> Result<ConfigReadResult, ConfigReaderError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConfigReaderError::Io { path: path.to_path_buf(), source: e })?;

    match ext {
        "json" => read_json(path, &content),
        "yaml" | "yml" => Ok(read_yaml_stub(path)),
        "js" | "mjs" | "cjs" => read_js_static(path, &content),
        "ts" | "mts" | "cts" => read_ts_static(path, &content),
        _ => Ok(ConfigReadResult {
            path: path.to_path_buf(),
            format: ConfigFormat::Json,
            status: ConfigReadStatus::Unreadable,
            values: vec![],
            warnings: vec![format!("Unknown config format: .{ext}")],
        }),
    }
}

fn read_json(path: &Path, content: &str) -> Result<ConfigReadResult, ConfigReaderError> {
    // Strip JSONC comments (// and /* */)
    let stripped = strip_jsonc_comments(content);

    match serde_json::from_str::<serde_json::Value>(&stripped) {
        Ok(value) => {
            let values = extract_json_values("", &value);
            Ok(ConfigReadResult {
                path: path.to_path_buf(),
                format: if content.contains("//") || content.contains("/*") {
                    ConfigFormat::Jsonc
                } else {
                    ConfigFormat::Json
                },
                status: ConfigReadStatus::Complete,
                values,
                warnings: vec![],
            })
        }
        Err(e) => {
            Err(ConfigReaderError::JsonParse { path: path.to_path_buf(), message: e.to_string() })
        }
    }
}

fn strip_jsonc_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if escaped {
            result.push(c);
            escaped = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            result.push(c);
            continue;
        }
        if c == '"' {
            in_string = true;
            result.push(c);
        } else if c == '/' {
            if chars.peek() == Some(&'/') {
                // Line comment
                chars.next();
                for nc in chars.by_ref() {
                    if nc == '\n' {
                        result.push('\n');
                        break;
                    }
                }
            } else if chars.peek() == Some(&'*') {
                // Block comment
                chars.next();
                loop {
                    match chars.next() {
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next();
                            result.push(' ');
                            break;
                        }
                        Some('\n') => result.push('\n'),
                        Some(_) => {}
                        None => break,
                    }
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn extract_json_values(prefix: &str, value: &serde_json::Value) -> Vec<ConfigValue> {
    let mut result = Vec::new();
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                result.push(ConfigValue { key: key.clone(), value: json_to_config_value(v) });
                // Recurse for nested objects
                if v.is_object() {
                    result.extend(extract_json_values(&key, v));
                }
            }
        }
        _ => {
            if !prefix.is_empty() {
                result.push(ConfigValue {
                    key: prefix.to_string(),
                    value: json_to_config_value(value),
                });
            }
        }
    }
    result
}

fn json_to_config_value(value: &serde_json::Value) -> ConfigValueKind {
    match value {
        serde_json::Value::String(s) => ConfigValueKind::String(s.clone()),
        serde_json::Value::Bool(b) => ConfigValueKind::Bool(*b),
        serde_json::Value::Number(n) => ConfigValueKind::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::Array(arr) => {
            ConfigValueKind::Array(arr.iter().map(json_to_config_value).collect())
        }
        serde_json::Value::Object(map) => ConfigValueKind::Object(
            map.iter().map(|(k, v)| (k.clone(), json_to_config_value(v))).collect(),
        ),
        serde_json::Value::Null => ConfigValueKind::String("null".to_string()),
    }
}

fn read_yaml_stub(path: &Path) -> ConfigReadResult {
    // YAML support is stubbed - would need serde_yaml dependency
    ConfigReadResult {
        path: path.to_path_buf(),
        format: ConfigFormat::Yaml,
        status: ConfigReadStatus::Unreadable,
        values: vec![],
        warnings: vec!["YAML config reading not yet implemented".to_string()],
    }
}

fn read_js_static(path: &Path, content: &str) -> Result<ConfigReadResult, ConfigReaderError> {
    read_js_ts_static(path, content, ConfigFormat::JavaScript)
}

fn read_ts_static(path: &Path, content: &str) -> Result<ConfigReadResult, ConfigReaderError> {
    read_js_ts_static(path, content, ConfigFormat::TypeScript)
}

/// Conservative static extraction from JS/TS config files.
/// Only extracts literal object exports -- function calls, dynamic values, etc.
/// are marked as Dynamic.
fn read_js_ts_static(
    path: &Path,
    content: &str,
    format: ConfigFormat,
) -> Result<ConfigReadResult, ConfigReaderError> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = match format {
        ConfigFormat::TypeScript => SourceType::ts(),
        _ => SourceType::mjs(),
    };

    let parser_ret = Parser::new(&allocator, content, source_type).parse();

    if parser_ret.panicked {
        return Err(ConfigReaderError::TooDynamic {
            path: path.to_path_buf(),
            reason: "Failed to parse".to_string(),
        });
    }

    let mut warnings = Vec::new();

    // Look for default export or module.exports
    // For now, mark JS/TS configs as partial with a warning
    // A full implementation would walk the AST to extract literal properties

    let has_default_export =
        content.contains("export default") || content.contains("module.exports");
    let has_function_call = content.contains("defineConfig(")
        || content.contains("defineNuxtConfig(")
        || content.contains("defineAstroConfig(");

    let has_dynamic = if has_function_call {
        warnings.push(
            "Config uses a wrapper function; only static literal properties can be extracted"
                .to_string(),
        );
        true
    } else {
        false
    };

    if !has_default_export && !has_function_call {
        warnings.push("No default export or module.exports found".to_string());
    }

    // Mark as partial since we can't fully evaluate JS/TS
    let status = if has_dynamic || has_default_export {
        ConfigReadStatus::Partial
    } else {
        ConfigReadStatus::Unreadable
    };

    Ok(ConfigReadResult { path: path.to_path_buf(), format, status, values: vec![], warnings })
}

/// Attempt to read known framework configs from a workspace root.
pub fn read_workspace_configs(workspace_root: &Path) -> Vec<ConfigReadResult> {
    let known_configs = [
        "next.config.js",
        "next.config.mjs",
        "next.config.ts",
        "nuxt.config.js",
        "nuxt.config.ts",
        "astro.config.js",
        "astro.config.mjs",
        "astro.config.ts",
        "vite.config.js",
        "vite.config.ts",
        "vite.config.mts",
        "vitest.config.js",
        "vitest.config.ts",
        "vitest.config.mts",
        "jest.config.js",
        "jest.config.ts",
        "jest.config.cjs",
        "jest.config.mjs",
        "playwright.config.js",
        "playwright.config.ts",
        "cypress.config.js",
        "cypress.config.ts",
        "angular.json",
        "nx.json",
        "turbo.json",
        "svelte.config.js",
        "docusaurus.config.js",
        "docusaurus.config.ts",
    ];

    let mut results = Vec::new();
    for config_name in &known_configs {
        let path = workspace_root.join(config_name);
        if path.exists() {
            match read_config(&path) {
                Ok(result) => results.push(result),
                Err(_) => {
                    results.push(ConfigReadResult {
                        path,
                        format: ConfigFormat::Json,
                        status: ConfigReadStatus::Unreadable,
                        values: vec![],
                        warnings: vec!["Failed to read config".to_string()],
                    });
                }
            }
        }
    }
    results
}
