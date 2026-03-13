use std::path::PathBuf;

use crate::{ConfigReadResult, ConfigValue, ConfigValueKind};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Structured inputs extracted from framework configuration files.
/// These feed into the graph builder to improve entrypoint detection,
/// alias resolution, and dependency tracking.
#[derive(Debug, Clone, Default)]
pub struct ConfigInputs {
    /// Path aliases discovered from config (e.g. webpack resolve.alias, vite resolve.alias).
    pub aliases: Vec<AliasEntry>,
    /// Additional entrypoint paths discovered from config.
    pub entrypoints: Vec<PathBuf>,
    /// Directories to include as source roots.
    pub source_roots: Vec<PathBuf>,
    /// Patterns for files to ignore.
    pub ignore_patterns: Vec<String>,
    /// External packages (should not be resolved as local).
    pub externals: Vec<String>,
    /// Test file patterns discovered from config.
    pub test_patterns: Vec<String>,
    /// Framework name that this config belongs to.
    pub framework: Option<String>,
}

/// A path alias mapping (e.g. `@/` -> `./src/`).
#[derive(Debug, Clone)]
pub struct AliasEntry {
    pub pattern: String,
    pub target: String,
}

impl ConfigInputs {
    /// Merge another `ConfigInputs` into this one, combining all fields.
    pub fn merge(&mut self, other: ConfigInputs) {
        self.aliases.extend(other.aliases);
        self.entrypoints.extend(other.entrypoints);
        self.source_roots.extend(other.source_roots);
        self.ignore_patterns.extend(other.ignore_patterns);
        self.externals.extend(other.externals);
        self.test_patterns.extend(other.test_patterns);
        // Keep the first framework name we see; don't overwrite with None.
        if self.framework.is_none() {
            self.framework = other.framework;
        }
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Trait for framework-specific config adapters.
pub trait ConfigAdapter {
    /// The framework name this adapter handles.
    fn framework_name(&self) -> &str;

    /// Check if this adapter can handle the given config file.
    fn matches(&self, config: &ConfigReadResult) -> bool;

    /// Extract structured inputs from the config read result.
    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a single config value by exact key.
fn find_value<'a>(values: &'a [ConfigValue], key: &str) -> Option<&'a ConfigValueKind> {
    values.iter().find(|v| v.key == key).map(|v| &v.value)
}

/// Find all config values whose key starts with `prefix`.
fn find_values_with_prefix<'a>(values: &'a [ConfigValue], prefix: &str) -> Vec<&'a ConfigValue> {
    values.iter().filter(|v| v.key.starts_with(prefix)).collect()
}

/// Extract a list of strings from a `ConfigValueKind::Array`.
fn strings_from_array(kind: &ConfigValueKind) -> Vec<String> {
    if let ConfigValueKind::Array(items) = kind {
        items
            .iter()
            .filter_map(|item| match item {
                ConfigValueKind::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    } else {
        vec![]
    }
}

/// Returns `true` if the file-name component of `path` starts with `stem`.
fn filename_starts_with(path: &std::path::Path, stem: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map_or(false, |n| n.starts_with(stem))
}

/// Returns `true` if *any* ancestor directory of `path` is named `dir`.
fn path_contains_dir(path: &std::path::Path, dir: &str) -> bool {
    path.components().any(|c| c.as_os_str() == dir)
}

// ---------------------------------------------------------------------------
// ViteAdapter
// ---------------------------------------------------------------------------

pub struct ViteAdapter;

impl ConfigAdapter for ViteAdapter {
    fn framework_name(&self) -> &str {
        "vite"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "vite.config")
            || filename_starts_with(&config.path, "vitest.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // resolve.alias — could be an object with key-value pairs
        let alias_entries = find_values_with_prefix(values, "resolve.alias.");
        for entry in alias_entries {
            // Keys look like "resolve.alias.@" with a String value target.
            let alias_key = entry.key.strip_prefix("resolve.alias.").unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        // If resolve.alias itself is an Array of objects [{find, replacement}], handle that too.
        if let Some(ConfigValueKind::Array(items)) = find_value(values, "resolve.alias") {
            for item in items {
                if let ConfigValueKind::Object(pairs) = item {
                    let find = pairs
                        .iter()
                        .find(|(k, _)| k == "find")
                        .and_then(|(_, v)| match v {
                            ConfigValueKind::String(s) => Some(s.clone()),
                            _ => None,
                        });
                    let replacement = pairs
                        .iter()
                        .find(|(k, _)| k == "replacement")
                        .and_then(|(_, v)| match v {
                            ConfigValueKind::String(s) => Some(s.clone()),
                            _ => None,
                        });
                    if let (Some(f), Some(r)) = (find, replacement) {
                        inputs.aliases.push(AliasEntry { pattern: f, target: r });
                    }
                }
            }
        }

        // build.rollupOptions.input — can be string or array or object
        if let Some(kind) = find_value(values, "build.rollupOptions.input") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.entrypoints.push(PathBuf::from(s));
                }
                ConfigValueKind::Array(items) => {
                    for item in items {
                        if let ConfigValueKind::String(s) = item {
                            inputs.entrypoints.push(PathBuf::from(s));
                        }
                    }
                }
                ConfigValueKind::Object(pairs) => {
                    for (_, v) in pairs {
                        if let ConfigValueKind::String(s) = v {
                            inputs.entrypoints.push(PathBuf::from(s));
                        }
                    }
                }
                _ => {}
            }
        }

        // Also look for rollup input entries stored as flat keys
        let input_entries = find_values_with_prefix(values, "build.rollupOptions.input.");
        for entry in input_entries {
            if let ConfigValueKind::String(s) = &entry.value {
                inputs.entrypoints.push(PathBuf::from(s));
            }
        }

        // test.include (vitest)
        if let Some(kind) = find_value(values, "test.include") {
            inputs.test_patterns.extend(strings_from_array(kind));
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// NextAdapter
// ---------------------------------------------------------------------------

pub struct NextAdapter;

impl ConfigAdapter for NextAdapter {
    fn framework_name(&self) -> &str {
        "next"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "next.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // pageExtensions → treated as entrypoint-related patterns (e.g. ["tsx", "ts", "jsx", "js"])
        if let Some(kind) = find_value(values, "pageExtensions") {
            for ext in strings_from_array(kind) {
                inputs.test_patterns.push(format!("**/*.{ext}"));
            }
        }

        // experimental.serverComponentsExternalPackages → externals
        if let Some(kind) = find_value(values, "experimental.serverComponentsExternalPackages") {
            inputs.externals.extend(strings_from_array(kind));
        }

        // serverExternalPackages (Next 15+)
        if let Some(kind) = find_value(values, "serverExternalPackages") {
            inputs.externals.extend(strings_from_array(kind));
        }

        // transpilePackages — not externals, but good to know about
        // (skipped: not directly useful to ConfigInputs fields)

        // webpack — typically a function, so it'll be Dynamic. We check for
        // any statically-extracted alias-like sub-keys anyway.
        let webpack_alias_entries = find_values_with_prefix(values, "webpack.resolve.alias.");
        for entry in webpack_alias_entries {
            let alias_key = entry
                .key
                .strip_prefix("webpack.resolve.alias.")
                .unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// JestAdapter
// ---------------------------------------------------------------------------

pub struct JestAdapter;

impl ConfigAdapter for JestAdapter {
    fn framework_name(&self) -> &str {
        "jest"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "jest.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // moduleNameMapper — an object where keys are regex patterns and values are replacement paths.
        // Stored as flat keys like "moduleNameMapper.^@/(.*)$".
        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "moduleNameMapper") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        // Also try flat-key style
        let mapper_entries = find_values_with_prefix(values, "moduleNameMapper.");
        for entry in mapper_entries {
            let pattern = entry
                .key
                .strip_prefix("moduleNameMapper.")
                .unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: pattern.to_string(),
                    target: target.clone(),
                });
            }
        }

        // testMatch
        if let Some(kind) = find_value(values, "testMatch") {
            inputs.test_patterns.extend(strings_from_array(kind));
        }

        // testPathPattern (string, not array)
        if let Some(ConfigValueKind::String(s)) = find_value(values, "testPathPattern") {
            inputs.test_patterns.push(s.clone());
        }

        // testRegex (string or array)
        if let Some(kind) = find_value(values, "testRegex") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.test_patterns.push(s.clone());
                }
                ConfigValueKind::Array(_) => {
                    inputs.test_patterns.extend(strings_from_array(kind));
                }
                _ => {}
            }
        }

        // roots
        if let Some(kind) = find_value(values, "roots") {
            for root in strings_from_array(kind) {
                inputs.source_roots.push(PathBuf::from(root));
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// PlaywrightAdapter
// ---------------------------------------------------------------------------

pub struct PlaywrightAdapter;

impl ConfigAdapter for PlaywrightAdapter {
    fn framework_name(&self) -> &str {
        "playwright"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "playwright.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // testDir
        if let Some(ConfigValueKind::String(dir)) = find_value(values, "testDir") {
            inputs.source_roots.push(PathBuf::from(dir));
        }

        // testMatch — string or array of glob patterns
        if let Some(kind) = find_value(values, "testMatch") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.test_patterns.push(s.clone());
                }
                ConfigValueKind::Array(_) => {
                    inputs.test_patterns.extend(strings_from_array(kind));
                }
                _ => {}
            }
        }

        // testIgnore
        if let Some(kind) = find_value(values, "testIgnore") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.ignore_patterns.push(s.clone());
                }
                ConfigValueKind::Array(_) => {
                    inputs.ignore_patterns.extend(strings_from_array(kind));
                }
                _ => {}
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// StorybookAdapter
// ---------------------------------------------------------------------------

pub struct StorybookAdapter;

impl ConfigAdapter for StorybookAdapter {
    fn framework_name(&self) -> &str {
        "storybook"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        path_contains_dir(&config.path, ".storybook")
            && (filename_starts_with(&config.path, "main.") ||
                // Also accept storybook main config filenames
                filename_starts_with(&config.path, "main"))
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // stories — array of glob patterns for story files
        if let Some(kind) = find_value(values, "stories") {
            inputs.test_patterns.extend(strings_from_array(kind));
        }

        // addons — array of package names
        if let Some(kind) = find_value(values, "addons") {
            inputs.externals.extend(strings_from_array(kind));
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// WebpackAdapter
// ---------------------------------------------------------------------------

pub struct WebpackAdapter;

impl ConfigAdapter for WebpackAdapter {
    fn framework_name(&self) -> &str {
        "webpack"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "webpack.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // resolve.alias — object with pattern → target
        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "resolve.alias") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        // Also try flat-key style (resolve.alias.@components, etc.)
        let alias_entries = find_values_with_prefix(values, "resolve.alias.");
        for entry in alias_entries {
            let alias_key = entry
                .key
                .strip_prefix("resolve.alias.")
                .unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        // entry — string, array, or object with named entries
        if let Some(kind) = find_value(values, "entry") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.entrypoints.push(PathBuf::from(s));
                }
                ConfigValueKind::Array(items) => {
                    for item in items {
                        if let ConfigValueKind::String(s) = item {
                            inputs.entrypoints.push(PathBuf::from(s));
                        }
                    }
                }
                ConfigValueKind::Object(pairs) => {
                    for (_, v) in pairs {
                        if let ConfigValueKind::String(s) = v {
                            inputs.entrypoints.push(PathBuf::from(s));
                        }
                    }
                }
                _ => {}
            }
        }
        // Flat-key entry sub-keys (entry.main, entry.vendor, etc.)
        let entry_entries = find_values_with_prefix(values, "entry.");
        for entry in entry_entries {
            if let ConfigValueKind::String(s) = &entry.value {
                inputs.entrypoints.push(PathBuf::from(s));
            }
        }

        // externals — string, array of strings, or object
        if let Some(kind) = find_value(values, "externals") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.externals.push(s.clone());
                }
                ConfigValueKind::Array(items) => {
                    for item in items {
                        if let ConfigValueKind::String(s) = item {
                            inputs.externals.push(s.clone());
                        }
                    }
                }
                ConfigValueKind::Object(pairs) => {
                    for (name, _) in pairs {
                        inputs.externals.push(name.clone());
                    }
                }
                _ => {}
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// Aggregate extraction
// ---------------------------------------------------------------------------

/// Run all registered adapters against workspace configs and merge results.
pub fn extract_all_inputs(configs: &[ConfigReadResult]) -> ConfigInputs {
    let adapters: Vec<Box<dyn ConfigAdapter>> = vec![
        Box::new(ViteAdapter),
        Box::new(NextAdapter),
        Box::new(JestAdapter),
        Box::new(PlaywrightAdapter),
        Box::new(StorybookAdapter),
        Box::new(WebpackAdapter),
    ];

    let mut merged = ConfigInputs::default();
    for config in configs {
        for adapter in &adapters {
            if adapter.matches(config) {
                let inputs = adapter.extract(config);
                merged.merge(inputs);
            }
        }
    }
    merged
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigFormat, ConfigReadStatus};

    /// Helper: build a minimal `ConfigReadResult` for testing.
    fn make_config(path: &str, values: Vec<ConfigValue>) -> ConfigReadResult {
        ConfigReadResult {
            path: PathBuf::from(path),
            format: ConfigFormat::JavaScript,
            status: ConfigReadStatus::Complete,
            values,
            warnings: vec![],
        }
    }

    fn cv(key: &str, value: ConfigValueKind) -> ConfigValue {
        ConfigValue { key: key.to_string(), value }
    }

    #[test]
    fn vite_adapter_extracts_aliases_from_flat_keys() {
        let config = make_config(
            "vite.config.ts",
            vec![
                cv("resolve.alias.@", ConfigValueKind::String("./src".to_string())),
                cv("resolve.alias.@utils", ConfigValueKind::String("./src/utils".to_string())),
            ],
        );
        let adapter = ViteAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.aliases.len(), 2);
        assert_eq!(inputs.aliases[0].pattern, "@");
        assert_eq!(inputs.aliases[0].target, "./src");
        assert_eq!(inputs.framework, Some("vite".to_string()));
    }

    #[test]
    fn vite_adapter_extracts_rollup_input() {
        let config = make_config(
            "vite.config.js",
            vec![cv(
                "build.rollupOptions.input",
                ConfigValueKind::Array(vec![
                    ConfigValueKind::String("src/main.ts".to_string()),
                    ConfigValueKind::String("src/other.ts".to_string()),
                ]),
            )],
        );
        let inputs = ViteAdapter.extract(&config);
        assert_eq!(inputs.entrypoints.len(), 2);
        assert_eq!(inputs.entrypoints[0], PathBuf::from("src/main.ts"));
    }

    #[test]
    fn next_adapter_extracts_externals() {
        let config = make_config(
            "next.config.mjs",
            vec![cv(
                "experimental.serverComponentsExternalPackages",
                ConfigValueKind::Array(vec![
                    ConfigValueKind::String("sharp".to_string()),
                    ConfigValueKind::String("canvas".to_string()),
                ]),
            )],
        );
        let adapter = NextAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.externals, vec!["sharp", "canvas"]);
    }

    #[test]
    fn jest_adapter_extracts_module_name_mapper_flat() {
        let config = make_config(
            "jest.config.ts",
            vec![
                cv(
                    "moduleNameMapper.^@/(.*)$",
                    ConfigValueKind::String("<rootDir>/src/$1".to_string()),
                ),
                cv(
                    "testMatch",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "**/__tests__/**/*.[jt]s?(x)".to_string(),
                    )]),
                ),
                cv(
                    "roots",
                    ConfigValueKind::Array(vec![ConfigValueKind::String("<rootDir>/src".to_string())]),
                ),
            ],
        );
        let adapter = JestAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "^@/(.*)$");
        assert_eq!(inputs.test_patterns, vec!["**/__tests__/**/*.[jt]s?(x)"]);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("<rootDir>/src")]);
    }

    #[test]
    fn playwright_adapter_extracts_test_dir() {
        let config = make_config(
            "playwright.config.ts",
            vec![
                cv("testDir", ConfigValueKind::String("./e2e".to_string())),
                cv(
                    "testMatch",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "**/*.spec.ts".to_string(),
                    )]),
                ),
            ],
        );
        let adapter = PlaywrightAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("./e2e")]);
        assert_eq!(inputs.test_patterns, vec!["**/*.spec.ts"]);
    }

    #[test]
    fn storybook_adapter_extracts_stories_and_addons() {
        let config = make_config(
            ".storybook/main.ts",
            vec![
                cv(
                    "stories",
                    ConfigValueKind::Array(vec![
                        ConfigValueKind::String("../src/**/*.stories.@(js|jsx|ts|tsx)".to_string()),
                    ]),
                ),
                cv(
                    "addons",
                    ConfigValueKind::Array(vec![
                        ConfigValueKind::String("@storybook/addon-essentials".to_string()),
                        ConfigValueKind::String("@storybook/addon-interactions".to_string()),
                    ]),
                ),
            ],
        );
        let adapter = StorybookAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.test_patterns, vec!["../src/**/*.stories.@(js|jsx|ts|tsx)"]);
        assert_eq!(
            inputs.externals,
            vec!["@storybook/addon-essentials", "@storybook/addon-interactions"]
        );
    }

    #[test]
    fn webpack_adapter_extracts_entry_and_aliases() {
        let config = make_config(
            "webpack.config.js",
            vec![
                cv("entry", ConfigValueKind::String("./src/index.js".to_string())),
                cv("resolve.alias.@components", ConfigValueKind::String("./src/components".to_string())),
                cv(
                    "externals",
                    ConfigValueKind::Array(vec![ConfigValueKind::String("jquery".to_string())]),
                ),
            ],
        );
        let adapter = WebpackAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.entrypoints, vec![PathBuf::from("./src/index.js")]);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@components");
        assert_eq!(inputs.externals, vec!["jquery"]);
    }

    #[test]
    fn extract_all_inputs_merges_multiple_configs() {
        let configs = vec![
            make_config(
                "vite.config.ts",
                vec![cv("resolve.alias.@", ConfigValueKind::String("./src".to_string()))],
            ),
            make_config(
                "jest.config.ts",
                vec![cv(
                    "testMatch",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "**/*.test.ts".to_string(),
                    )]),
                )],
            ),
        ];
        let merged = extract_all_inputs(&configs);
        assert_eq!(merged.aliases.len(), 1);
        assert_eq!(merged.test_patterns, vec!["**/*.test.ts"]);
        // First matched framework wins
        assert_eq!(merged.framework, Some("vite".to_string()));
    }

    #[test]
    fn adapter_does_not_match_wrong_config() {
        let config = make_config("tsconfig.json", vec![]);
        assert!(!ViteAdapter.matches(&config));
        assert!(!NextAdapter.matches(&config));
        assert!(!JestAdapter.matches(&config));
        assert!(!PlaywrightAdapter.matches(&config));
        assert!(!StorybookAdapter.matches(&config));
        assert!(!WebpackAdapter.matches(&config));
    }

    #[test]
    fn merge_preserves_first_framework() {
        let mut a = ConfigInputs {
            framework: Some("vite".to_string()),
            ..Default::default()
        };
        let b = ConfigInputs {
            framework: Some("jest".to_string()),
            externals: vec!["foo".to_string()],
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.framework, Some("vite".to_string()));
        assert_eq!(a.externals, vec!["foo"]);
    }

    #[test]
    fn merge_takes_framework_when_self_is_none() {
        let mut a = ConfigInputs::default();
        let b = ConfigInputs {
            framework: Some("next".to_string()),
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.framework, Some("next".to_string()));
    }

    #[test]
    fn vite_adapter_extracts_alias_array_form() {
        let config = make_config(
            "vite.config.ts",
            vec![cv(
                "resolve.alias",
                ConfigValueKind::Array(vec![ConfigValueKind::Object(vec![
                    ("find".to_string(), ConfigValueKind::String("@".to_string())),
                    (
                        "replacement".to_string(),
                        ConfigValueKind::String("./src".to_string()),
                    ),
                ])]),
            )],
        );
        let inputs = ViteAdapter.extract(&config);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@");
        assert_eq!(inputs.aliases[0].target, "./src");
    }

    #[test]
    fn vitest_config_matches_vite_adapter() {
        let config = make_config(
            "vitest.config.ts",
            vec![cv(
                "test.include",
                ConfigValueKind::Array(vec![ConfigValueKind::String(
                    "**/*.{test,spec}.{js,ts}".to_string(),
                )]),
            )],
        );
        assert!(ViteAdapter.matches(&config));
        let inputs = ViteAdapter.extract(&config);
        assert_eq!(inputs.test_patterns, vec!["**/*.{test,spec}.{js,ts}"]);
    }
}
