pub mod adapters;

use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

pub use adapters::{AliasEntry, ConfigAdapter, ConfigInputs, extract_all_inputs};

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
        "yaml" | "yml" => read_yaml(path, &content),
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

// ---------------------------------------------------------------------------
// YAML reading
// ---------------------------------------------------------------------------

fn read_yaml(path: &Path, content: &str) -> Result<ConfigReadResult, ConfigReaderError> {
    match serde_yaml::from_str::<serde_yaml::Value>(content) {
        Ok(value) => {
            let json_value = yaml_to_json(&value);
            let values = extract_json_values("", &json_value);
            Ok(ConfigReadResult {
                path: path.to_path_buf(),
                format: ConfigFormat::Yaml,
                status: ConfigReadStatus::Complete,
                values,
                warnings: vec![],
            })
        }
        Err(e) => Err(ConfigReaderError::JsonParse {
            path: path.to_path_buf(),
            message: format!("YAML parse error: {e}"),
        }),
    }
}

fn yaml_to_json(value: &serde_yaml::Value) -> serde_json::Value {
    match value {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(serde_json::Number::from(i))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map_or(serde_json::Value::Null, serde_json::Value::Number)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter_map(|(k, v)| k.as_str().map(|key| (key.to_string(), yaml_to_json(v))))
                .collect();
            serde_json::Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

// ---------------------------------------------------------------------------
// JS/TS static extraction
// ---------------------------------------------------------------------------

fn read_js_static(path: &Path, content: &str) -> Result<ConfigReadResult, ConfigReaderError> {
    read_js_ts_static(path, content, ConfigFormat::JavaScript)
}

fn read_ts_static(path: &Path, content: &str) -> Result<ConfigReadResult, ConfigReaderError> {
    read_js_ts_static(path, content, ConfigFormat::TypeScript)
}

/// Conservative static extraction from JS/TS config files.
/// Parses the file with oxc, finds the default export (or `module.exports`),
/// and extracts literal properties from object expressions. Dynamic values
/// are marked as `ConfigValueKind::Dynamic`.
fn read_js_ts_static(
    path: &Path,
    content: &str,
    format: ConfigFormat,
) -> Result<ConfigReadResult, ConfigReaderError> {
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{ExportDefaultDeclarationKind, Expression, Statement};
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

    let program = &parser_ret.program;
    let mut values = Vec::new();
    let mut warnings = Vec::new();
    let mut found_export = false;

    for stmt in &program.body {
        // 1. export default <expr>
        if let Statement::ExportDefaultDeclaration(export) = stmt {
            found_export = true;
            match &export.declaration {
                ExportDefaultDeclarationKind::ObjectExpression(obj) => {
                    extract_object_expression("", obj, &mut values, &mut warnings);
                }
                ExportDefaultDeclarationKind::CallExpression(call) => {
                    // e.g. export default defineConfig({...})
                    extract_from_call_expression(call, &mut values, &mut warnings);
                }
                ExportDefaultDeclarationKind::FunctionDeclaration(_)
                | ExportDefaultDeclarationKind::ArrowFunctionExpression(_) => {
                    warnings.push(
                        "Default export is a function; cannot extract statically".to_string(),
                    );
                }
                ExportDefaultDeclarationKind::Identifier(_) => {
                    warnings.push(
                        "Default export is an identifier reference; cannot resolve statically"
                            .to_string(),
                    );
                }
                ExportDefaultDeclarationKind::TSAsExpression(ts_as) => {
                    // export default { ... } as const  /  export default { ... } satisfies Config
                    extract_from_expression(&ts_as.expression, &mut values, &mut warnings);
                }
                ExportDefaultDeclarationKind::TSSatisfiesExpression(ts_sat) => {
                    extract_from_expression(&ts_sat.expression, &mut values, &mut warnings);
                }
                ExportDefaultDeclarationKind::ParenthesizedExpression(paren) => {
                    extract_from_expression(&paren.expression, &mut values, &mut warnings);
                }
                _ => {
                    warnings.push("Default export has an unsupported form".to_string());
                }
            }
        }

        // 2. module.exports = <expr>
        if let Statement::ExpressionStatement(expr_stmt) = stmt
            && let Expression::AssignmentExpression(assign) = &expr_stmt.expression
            && is_module_exports_target(&assign.left)
        {
            found_export = true;
            extract_from_expression(&assign.right, &mut values, &mut warnings);
        }
    }

    if !found_export {
        warnings.push("No default export or module.exports found".to_string());
    }

    let status = if !found_export {
        ConfigReadStatus::Unreadable
    } else if warnings.is_empty() {
        ConfigReadStatus::Complete
    } else {
        ConfigReadStatus::Partial
    };

    Ok(ConfigReadResult { path: path.to_path_buf(), format, status, values, warnings })
}

/// Check if an assignment target is `module.exports`.
fn is_module_exports_target(target: &oxc_ast::ast::AssignmentTarget<'_>) -> bool {
    if let oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) = target
        && member.property.name.as_str() == "exports"
        && let oxc_ast::ast::Expression::Identifier(ident) = &member.object
    {
        return ident.name.as_str() == "module";
    }
    false
}

/// Extract config values from an arbitrary expression (dispatches to the
/// appropriate handler based on expression kind).
fn extract_from_expression(
    expr: &oxc_ast::ast::Expression<'_>,
    values: &mut Vec<ConfigValue>,
    warnings: &mut Vec<String>,
) {
    use oxc_ast::ast::Expression;
    match expr {
        Expression::ObjectExpression(obj) => {
            extract_object_expression("", obj, values, warnings);
        }
        Expression::CallExpression(call) => {
            extract_from_call_expression(call, values, warnings);
        }
        Expression::ParenthesizedExpression(paren) => {
            extract_from_expression(&paren.expression, values, warnings);
        }
        _ => {
            warnings.push("Export value is not a static object literal".to_string());
        }
    }
}

/// Extract the object argument from a wrapper call like `defineConfig({...})`.
fn extract_from_call_expression(
    call: &oxc_ast::ast::CallExpression<'_>,
    values: &mut Vec<ConfigValue>,
    warnings: &mut Vec<String>,
) {
    // Try to get the wrapper function name for a better warning message
    let callee_name = match &call.callee {
        oxc_ast::ast::Expression::Identifier(ident) => Some(ident.name.to_string()),
        _ => None,
    };

    if let Some(name) = &callee_name {
        warnings.push(format!(
            "Config uses `{name}()` wrapper; only static literal properties can be extracted"
        ));
    } else {
        warnings.push(
            "Config uses a function call wrapper; only static literal properties can be extracted"
                .to_string(),
        );
    }

    // Look for the first object argument
    for arg in &call.arguments {
        if let oxc_ast::ast::Argument::ObjectExpression(obj) = arg {
            extract_object_expression("", obj, values, warnings);
            return;
        }
    }

    warnings.push("Could not find an object literal argument in the wrapper call".to_string());
}

/// Recursively extract properties from an object expression.
fn extract_object_expression(
    prefix: &str,
    obj: &oxc_ast::ast::ObjectExpression<'_>,
    values: &mut Vec<ConfigValue>,
    warnings: &mut Vec<String>,
) {
    use oxc_ast::ast::{Expression, ObjectPropertyKind, PropertyKey};

    for prop_kind in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop_kind {
            let key = match &prop.key {
                PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                PropertyKey::StringLiteral(lit) => Some(lit.value.to_string()),
                _ => None,
            };

            if let Some(key_name) = key {
                let full_key = if prefix.is_empty() {
                    key_name.clone()
                } else {
                    format!("{prefix}.{key_name}")
                };

                match expression_to_config_value(&prop.value) {
                    Ok(val) => {
                        values.push(ConfigValue { key: full_key.clone(), value: val });
                        // Recurse for nested objects
                        if let Expression::ObjectExpression(nested) = &prop.value {
                            extract_object_expression(&full_key, nested, values, warnings);
                        }
                    }
                    Err(desc) => {
                        values.push(ConfigValue {
                            key: full_key,
                            value: ConfigValueKind::Dynamic(desc),
                        });
                        warnings.push(format!("Property '{key_name}' has a dynamic value"));
                    }
                }
            } else {
                warnings.push("Skipping property with computed key".to_string());
            }
        } else {
            // SpreadProperty
            warnings.push("Skipping spread property in object".to_string());
        }
    }
}

/// Try to convert an expression to a static `ConfigValueKind`.
/// Returns `Err(description)` if the expression is dynamic.
fn expression_to_config_value(
    expr: &oxc_ast::ast::Expression<'_>,
) -> Result<ConfigValueKind, String> {
    use oxc_ast::ast::Expression;

    match expr {
        Expression::StringLiteral(lit) => Ok(ConfigValueKind::String(lit.value.to_string())),
        Expression::NumericLiteral(lit) => Ok(ConfigValueKind::Number(lit.value)),
        Expression::BooleanLiteral(lit) => Ok(ConfigValueKind::Bool(lit.value)),
        Expression::NullLiteral(_) => Ok(ConfigValueKind::String("null".to_string())),
        Expression::TemplateLiteral(tpl) => {
            // Only extract if it has no expressions (i.e. a simple string)
            if tpl.expressions.is_empty() {
                if let Some(quasi) = tpl.quasis.first() {
                    Ok(ConfigValueKind::String(quasi.value.raw.to_string()))
                } else {
                    Ok(ConfigValueKind::String(String::new()))
                }
            } else {
                Err("template literal with expressions".to_string())
            }
        }
        Expression::ArrayExpression(arr) => {
            let mut items = Vec::new();
            for element in &arr.elements {
                match element {
                    oxc_ast::ast::ArrayExpressionElement::SpreadElement(_)
                    | oxc_ast::ast::ArrayExpressionElement::Elision(_) => {
                        // Skip spread elements and holes
                    }
                    // All other variants are inherited from Expression
                    other => {
                        if let Some(val) = try_array_element_to_config_value(other) {
                            items.push(val);
                        }
                    }
                }
            }
            Ok(ConfigValueKind::Array(items))
        }
        Expression::ObjectExpression(_) => {
            // The actual key-value pairs are extracted by the caller via recursion
            Ok(ConfigValueKind::Object(vec![]))
        }
        Expression::UnaryExpression(unary) => {
            // Handle negative numbers: -1, -3.14
            if unary.operator == oxc_ast::ast::UnaryOperator::UnaryNegation
                && let Expression::NumericLiteral(lit) = &unary.argument
            {
                return Ok(ConfigValueKind::Number(-lit.value));
            }
            Err("unary expression".to_string())
        }
        Expression::Identifier(ident) => {
            let name = ident.name.as_str();
            match name {
                "undefined" => Ok(ConfigValueKind::String("undefined".to_string())),
                "Infinity" => Ok(ConfigValueKind::Number(f64::INFINITY)),
                "NaN" => Ok(ConfigValueKind::Number(f64::NAN)),
                _ => Err(format!("identifier `{name}`")),
            }
        }
        _ => Err("dynamic expression".to_string()),
    }
}

/// Try to convert an `ArrayExpressionElement` (which inherits `Expression` variants)
/// into a `ConfigValueKind`.
fn try_array_element_to_config_value(
    element: &oxc_ast::ast::ArrayExpressionElement<'_>,
) -> Option<ConfigValueKind> {
    use oxc_ast::ast::ArrayExpressionElement;
    match element {
        ArrayExpressionElement::StringLiteral(lit) => {
            Some(ConfigValueKind::String(lit.value.to_string()))
        }
        ArrayExpressionElement::NumericLiteral(lit) => Some(ConfigValueKind::Number(lit.value)),
        ArrayExpressionElement::BooleanLiteral(lit) => Some(ConfigValueKind::Bool(lit.value)),
        ArrayExpressionElement::NullLiteral(_) => Some(ConfigValueKind::String("null".to_string())),
        ArrayExpressionElement::ObjectExpression(_) => Some(ConfigValueKind::Object(vec![])),
        ArrayExpressionElement::TemplateLiteral(tpl) => {
            if tpl.expressions.is_empty() {
                tpl.quasis.first().map(|quasi| ConfigValueKind::String(quasi.value.raw.to_string()))
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Workspace config discovery
// ---------------------------------------------------------------------------

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
        "svelte.config.ts",
        "docusaurus.config.js",
        "docusaurus.config.ts",
        ".storybook/main.ts",
        ".storybook/main.js",
        ".storybook/preview.ts",
        ".storybook/preview.js",
        "webpack.config.js",
        "webpack.config.ts",
        "webpack.config.cjs",
        "webpack.config.mjs",
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
