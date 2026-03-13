use std::path::PathBuf;

use crate::{ConfigReadResult, ConfigValue, ConfigValueKind};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Confidence level for an adapter's outputs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AdapterConfidence {
    #[default]
    High,
    Medium,
    Low,
}

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
    /// Runtime entrypoints (e.g. server entry, worker entry).
    pub runtime_entrypoints: Vec<PathBuf>,
    /// Production-only entrypoints.
    pub production_entrypoints: Vec<PathBuf>,
    /// Development-only entrypoints (e.g. dev server entry).
    pub development_entrypoints: Vec<PathBuf>,
    /// Story file entrypoints.
    pub story_entrypoints: Vec<PathBuf>,
    /// Setup files (e.g. jest setup, vitest setup).
    pub setup_files: Vec<PathBuf>,
    /// Global setup files.
    pub global_setup_files: Vec<PathBuf>,
    /// Auto-import root directories (e.g. Nuxt composables/).
    pub auto_import_roots: Vec<PathBuf>,
    /// Confidence level for this adapter's outputs.
    pub confidence: AdapterConfidence,
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
        self.runtime_entrypoints.extend(other.runtime_entrypoints);
        self.production_entrypoints.extend(other.production_entrypoints);
        self.development_entrypoints.extend(other.development_entrypoints);
        self.story_entrypoints.extend(other.story_entrypoints);
        self.setup_files.extend(other.setup_files);
        self.global_setup_files.extend(other.global_setup_files);
        self.auto_import_roots.extend(other.auto_import_roots);
        // Keep the first non-default confidence.
        if self.confidence == AdapterConfidence::High && other.confidence != AdapterConfidence::High
        {
            self.confidence = other.confidence;
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
// VitestAdapter
// ---------------------------------------------------------------------------

pub struct VitestAdapter;

impl ConfigAdapter for VitestAdapter {
    fn framework_name(&self) -> &str {
        "vitest"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "vitest.config")
            || filename_starts_with(&config.path, "vitest.workspace")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // test.include → test_patterns
        if let Some(kind) = find_value(values, "test.include") {
            inputs.test_patterns.extend(strings_from_array(kind));
        }

        // test.setupFiles → setup_files
        if let Some(kind) = find_value(values, "test.setupFiles") {
            for s in strings_from_array(kind) {
                inputs.setup_files.push(PathBuf::from(s));
            }
        }
        // Also handle single string form
        if let Some(ConfigValueKind::String(s)) = find_value(values, "test.setupFiles") {
            inputs.setup_files.push(PathBuf::from(s));
        }

        // test.globalSetup → global_setup_files
        if let Some(kind) = find_value(values, "test.globalSetup") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.global_setup_files.push(PathBuf::from(s));
                }
                ConfigValueKind::Array(_) => {
                    for s in strings_from_array(kind) {
                        inputs.global_setup_files.push(PathBuf::from(s));
                    }
                }
                _ => {}
            }
        }

        // resolve.alias → aliases
        let alias_entries = find_values_with_prefix(values, "resolve.alias.");
        for entry in alias_entries {
            let alias_key = entry.key.strip_prefix("resolve.alias.").unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        // Also handle resolve.alias as Array of objects [{find, replacement}]
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

        inputs
    }
}

// ---------------------------------------------------------------------------
// NuxtAdapter
// ---------------------------------------------------------------------------

pub struct NuxtAdapter;

impl ConfigAdapter for NuxtAdapter {
    fn framework_name(&self) -> &str {
        "nuxt"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "nuxt.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // alias → aliases (object with pattern → target)
        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "alias") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        let alias_entries = find_values_with_prefix(values, "alias.");
        for entry in alias_entries {
            let alias_key = entry.key.strip_prefix("alias.").unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        // imports.dirs → auto_import_roots
        if let Some(kind) = find_value(values, "imports.dirs") {
            for s in strings_from_array(kind) {
                inputs.auto_import_roots.push(PathBuf::from(s));
            }
        }

        // components.dirs → auto_import_roots
        if let Some(kind) = find_value(values, "components.dirs") {
            for s in strings_from_array(kind) {
                inputs.auto_import_roots.push(PathBuf::from(s));
            }
        }

        // dir.pages, dir.layouts, dir.middleware, dir.plugins → production_entrypoints
        for sub in &["dir.pages", "dir.layouts", "dir.middleware", "dir.plugins"] {
            if let Some(ConfigValueKind::String(s)) = find_value(values, sub) {
                inputs.production_entrypoints.push(PathBuf::from(s));
            }
        }

        // modules → externals
        if let Some(kind) = find_value(values, "modules") {
            inputs.externals.extend(strings_from_array(kind));
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// AstroAdapter
// ---------------------------------------------------------------------------

pub struct AstroAdapter;

impl ConfigAdapter for AstroAdapter {
    fn framework_name(&self) -> &str {
        "astro"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "astro.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // srcDir → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "srcDir") {
            inputs.source_roots.push(PathBuf::from(s));
        }

        // outDir → ignore_patterns
        if let Some(ConfigValueKind::String(s)) = find_value(values, "outDir") {
            inputs.ignore_patterns.push(s.clone());
        }

        // integrations — extract package names as externals
        if let Some(kind) = find_value(values, "integrations") {
            inputs.externals.extend(strings_from_array(kind));
        }

        // vite.resolve.alias → aliases
        let alias_entries = find_values_with_prefix(values, "vite.resolve.alias.");
        for entry in alias_entries {
            let alias_key = entry
                .key
                .strip_prefix("vite.resolve.alias.")
                .unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "vite.resolve.alias") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// SvelteKitAdapter
// ---------------------------------------------------------------------------

pub struct SvelteKitAdapter;

impl ConfigAdapter for SvelteKitAdapter {
    fn framework_name(&self) -> &str {
        "sveltekit"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "svelte.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // kit.files.routes → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "kit.files.routes") {
            inputs.source_roots.push(PathBuf::from(s));
        }

        // kit.files.lib → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "kit.files.lib") {
            inputs.source_roots.push(PathBuf::from(s));
        }

        // kit.alias → aliases (object)
        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "kit.alias") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        let alias_entries = find_values_with_prefix(values, "kit.alias.");
        for entry in alias_entries {
            let alias_key = entry.key.strip_prefix("kit.alias.").unwrap_or(&entry.key);
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
// RemixAdapter
// ---------------------------------------------------------------------------

pub struct RemixAdapter;

impl ConfigAdapter for RemixAdapter {
    fn framework_name(&self) -> &str {
        "remix"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "remix.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // appDirectory → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "appDirectory") {
            inputs.source_roots.push(PathBuf::from(s));
        }

        // routes → production_entrypoints (string values)
        if let Some(kind) = find_value(values, "routes") {
            for s in strings_from_array(kind) {
                inputs.production_entrypoints.push(PathBuf::from(s));
            }
        }
        // Also flat-key route entries
        let route_entries = find_values_with_prefix(values, "routes.");
        for entry in route_entries {
            if let ConfigValueKind::String(s) = &entry.value {
                inputs.production_entrypoints.push(PathBuf::from(s));
            }
        }

        // serverBuildPath → ignore_patterns
        if let Some(ConfigValueKind::String(s)) = find_value(values, "serverBuildPath") {
            inputs.ignore_patterns.push(s.clone());
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// AngularAdapter
// ---------------------------------------------------------------------------

pub struct AngularAdapter;

impl ConfigAdapter for AngularAdapter {
    fn framework_name(&self) -> &str {
        "angular"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "angular.json")
            || filename_starts_with(&config.path, "angular.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // projects.*.architect.build.options.main → entrypoints
        let build_main_entries =
            find_values_with_prefix(values, "projects.");
        for entry in &build_main_entries {
            if entry.key.contains(".architect.build.options.main") {
                if let ConfigValueKind::String(s) = &entry.value {
                    inputs.entrypoints.push(PathBuf::from(s));
                }
            }
            // projects.*.architect.build.options.styles → entrypoints
            if entry.key.contains(".architect.build.options.styles") {
                if let ConfigValueKind::Array(_) = &entry.value {
                    for s in strings_from_array(&entry.value) {
                        inputs.entrypoints.push(PathBuf::from(s));
                    }
                }
            }
            // projects.*.architect.build.options.scripts → entrypoints
            if entry.key.contains(".architect.build.options.scripts") {
                if let ConfigValueKind::Array(_) = &entry.value {
                    for s in strings_from_array(&entry.value) {
                        inputs.entrypoints.push(PathBuf::from(s));
                    }
                }
            }
            // projects.*.architect.test.options.main → setup_files
            if entry.key.contains(".architect.test.options.main") {
                if let ConfigValueKind::String(s) = &entry.value {
                    inputs.setup_files.push(PathBuf::from(s));
                }
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// NxAdapter
// ---------------------------------------------------------------------------

pub struct NxAdapter;

impl ConfigAdapter for NxAdapter {
    fn framework_name(&self) -> &str {
        "nx"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "nx.json")
            || filename_starts_with(&config.path, "project.json")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // targets.build.options.main → entrypoints
        if let Some(ConfigValueKind::String(s)) = find_value(values, "targets.build.options.main") {
            inputs.entrypoints.push(PathBuf::from(s));
        }

        // implicitDependencies → externals
        if let Some(kind) = find_value(values, "implicitDependencies") {
            inputs.externals.extend(strings_from_array(kind));
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// TurborepoAdapter
// ---------------------------------------------------------------------------

pub struct TurborepoAdapter;

impl ConfigAdapter for TurborepoAdapter {
    fn framework_name(&self) -> &str {
        "turborepo"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "turbo.json")
    }

    fn extract(&self, _config: &ConfigReadResult) -> ConfigInputs {
        // Limited static extraction — pipeline keys are mostly metadata.
        ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            confidence: AdapterConfidence::Medium,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// VitePressAdapter
// ---------------------------------------------------------------------------

pub struct VitePressAdapter;

impl ConfigAdapter for VitePressAdapter {
    fn framework_name(&self) -> &str {
        "vitepress"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "vitepress.config")
            || (path_contains_dir(&config.path, ".vitepress")
                && filename_starts_with(&config.path, "config"))
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // srcDir → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "srcDir") {
            inputs.source_roots.push(PathBuf::from(s));
        }

        // outDir → ignore_patterns
        if let Some(ConfigValueKind::String(s)) = find_value(values, "outDir") {
            inputs.ignore_patterns.push(s.clone());
        }

        // themeConfig → skip

        inputs
    }
}

// ---------------------------------------------------------------------------
// DocusaurusAdapter
// ---------------------------------------------------------------------------

pub struct DocusaurusAdapter;

impl ConfigAdapter for DocusaurusAdapter {
    fn framework_name(&self) -> &str {
        "docusaurus"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "docusaurus.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // presets → externals (package names)
        if let Some(kind) = find_value(values, "presets") {
            inputs.externals.extend(strings_from_array(kind));
        }

        // plugins → externals (package names)
        if let Some(kind) = find_value(values, "plugins") {
            inputs.externals.extend(strings_from_array(kind));
        }

        // customFields → skip

        inputs
    }
}

// ---------------------------------------------------------------------------
// RollupAdapter
// ---------------------------------------------------------------------------

pub struct RollupAdapter;

impl ConfigAdapter for RollupAdapter {
    fn framework_name(&self) -> &str {
        "rollup"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "rollup.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // input → entrypoints (string, array, or object)
        if let Some(kind) = find_value(values, "input") {
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

        // external → externals (string or array)
        if let Some(kind) = find_value(values, "external") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.externals.push(s.clone());
                }
                ConfigValueKind::Array(_) => {
                    inputs.externals.extend(strings_from_array(kind));
                }
                _ => {}
            }
        }

        // plugins — extract names as externals
        if let Some(kind) = find_value(values, "plugins") {
            inputs.externals.extend(strings_from_array(kind));
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// RspackAdapter
// ---------------------------------------------------------------------------

pub struct RspackAdapter;

impl ConfigAdapter for RspackAdapter {
    fn framework_name(&self) -> &str {
        "rspack"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "rspack.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // entry → entrypoints (string, array, or object)
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
        let entry_entries = find_values_with_prefix(values, "entry.");
        for entry in entry_entries {
            if let ConfigValueKind::String(s) = &entry.value {
                inputs.entrypoints.push(PathBuf::from(s));
            }
        }

        // resolve.alias → aliases
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

        // externals → externals
        if let Some(kind) = find_value(values, "externals") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.externals.push(s.clone());
                }
                ConfigValueKind::Array(_) => {
                    inputs.externals.extend(strings_from_array(kind));
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
// RsbuildAdapter
// ---------------------------------------------------------------------------

pub struct RsbuildAdapter;

impl ConfigAdapter for RsbuildAdapter {
    fn framework_name(&self) -> &str {
        "rsbuild"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "rsbuild.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // source.entry → entrypoints (object with named entries or string)
        if let Some(kind) = find_value(values, "source.entry") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.entrypoints.push(PathBuf::from(s));
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
        let entry_entries = find_values_with_prefix(values, "source.entry.");
        for entry in entry_entries {
            if let ConfigValueKind::String(s) = &entry.value {
                inputs.entrypoints.push(PathBuf::from(s));
            }
        }

        // source.alias → aliases
        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "source.alias") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        let alias_entries = find_values_with_prefix(values, "source.alias.");
        for entry in alias_entries {
            let alias_key = entry
                .key
                .strip_prefix("source.alias.")
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
// ParcelAdapter
// ---------------------------------------------------------------------------

pub struct ParcelAdapter;

impl ConfigAdapter for ParcelAdapter {
    fn framework_name(&self) -> &str {
        "parcel"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, ".parcelrc")
    }

    fn extract(&self, _config: &ConfigReadResult) -> ConfigInputs {
        // Limited static extraction from .parcelrc.
        ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            confidence: AdapterConfidence::Low,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// GatsbyAdapter
// ---------------------------------------------------------------------------

pub struct GatsbyAdapter;

impl ConfigAdapter for GatsbyAdapter {
    fn framework_name(&self) -> &str {
        "gatsby"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "gatsby-config")
            || filename_starts_with(&config.path, "gatsby-node")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // plugins → externals (package names)
        if let Some(kind) = find_value(values, "plugins") {
            inputs.externals.extend(strings_from_array(kind));
        }

        // siteMetadata → skip

        inputs
    }
}

// ---------------------------------------------------------------------------
// NitroAdapter
// ---------------------------------------------------------------------------

pub struct NitroAdapter;

impl ConfigAdapter for NitroAdapter {
    fn framework_name(&self) -> &str {
        "nitro"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "nitro.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // alias → aliases
        if let Some(ConfigValueKind::Object(pairs)) = find_value(values, "alias") {
            for (pattern, value) in pairs {
                if let ConfigValueKind::String(target) = value {
                    inputs.aliases.push(AliasEntry {
                        pattern: pattern.clone(),
                        target: target.clone(),
                    });
                }
            }
        }
        let alias_entries = find_values_with_prefix(values, "alias.");
        for entry in alias_entries {
            let alias_key = entry.key.strip_prefix("alias.").unwrap_or(&entry.key);
            if let ConfigValueKind::String(target) = &entry.value {
                inputs.aliases.push(AliasEntry {
                    pattern: alias_key.to_string(),
                    target: target.clone(),
                });
            }
        }

        // imports.dirs → auto_import_roots
        if let Some(kind) = find_value(values, "imports.dirs") {
            for s in strings_from_array(kind) {
                inputs.auto_import_roots.push(PathBuf::from(s));
            }
        }

        // externals → externals (array of strings)
        if let Some(kind) = find_value(values, "externals") {
            match kind {
                ConfigValueKind::String(s) => {
                    inputs.externals.push(s.clone());
                }
                ConfigValueKind::Array(_) => {
                    inputs.externals.extend(strings_from_array(kind));
                }
                _ => {}
            }
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// ReactRouterAdapter
// ---------------------------------------------------------------------------

pub struct ReactRouterAdapter;

impl ConfigAdapter for ReactRouterAdapter {
    fn framework_name(&self) -> &str {
        "react-router"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "react-router.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            ..Default::default()
        };

        // appDirectory → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "appDirectory") {
            inputs.source_roots.push(PathBuf::from(s));
        }

        // routes → production_entrypoints
        if let Some(kind) = find_value(values, "routes") {
            for s in strings_from_array(kind) {
                inputs.production_entrypoints.push(PathBuf::from(s));
            }
        }
        let route_entries = find_values_with_prefix(values, "routes.");
        for entry in route_entries {
            if let ConfigValueKind::String(s) = &entry.value {
                inputs.production_entrypoints.push(PathBuf::from(s));
            }
        }

        // serverBuildPath → ignore_patterns
        if let Some(ConfigValueKind::String(s)) = find_value(values, "serverBuildPath") {
            inputs.ignore_patterns.push(s.clone());
        }

        inputs
    }
}

// ---------------------------------------------------------------------------
// QwikAdapter
// ---------------------------------------------------------------------------

pub struct QwikAdapter;

impl ConfigAdapter for QwikAdapter {
    fn framework_name(&self) -> &str {
        "qwik"
    }

    fn matches(&self, config: &ConfigReadResult) -> bool {
        filename_starts_with(&config.path, "qwik.config")
    }

    fn extract(&self, config: &ConfigReadResult) -> ConfigInputs {
        let values = &config.values;
        let mut inputs = ConfigInputs {
            framework: Some(self.framework_name().to_string()),
            confidence: AdapterConfidence::Medium,
            ..Default::default()
        };

        // srcDir → source_roots
        if let Some(ConfigValueKind::String(s)) = find_value(values, "srcDir") {
            inputs.source_roots.push(PathBuf::from(s));
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
        // Tier A
        Box::new(VitestAdapter),
        Box::new(NuxtAdapter),
        Box::new(AstroAdapter),
        Box::new(SvelteKitAdapter),
        Box::new(RemixAdapter),
        Box::new(AngularAdapter),
        Box::new(NxAdapter),
        Box::new(TurborepoAdapter),
        Box::new(VitePressAdapter),
        Box::new(DocusaurusAdapter),
        // Tier B
        Box::new(RollupAdapter),
        Box::new(RspackAdapter),
        Box::new(RsbuildAdapter),
        Box::new(ParcelAdapter),
        Box::new(GatsbyAdapter),
        Box::new(NitroAdapter),
        Box::new(ReactRouterAdapter),
        Box::new(QwikAdapter),
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

    // -------------------------------------------------------------------
    // New adapter tests
    // -------------------------------------------------------------------

    #[test]
    fn vitest_adapter_extracts_test_patterns_and_setup() {
        let config = make_config(
            "vitest.config.ts",
            vec![
                cv(
                    "test.include",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "**/*.{test,spec}.ts".to_string(),
                    )]),
                ),
                cv(
                    "test.setupFiles",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "./test/setup.ts".to_string(),
                    )]),
                ),
                cv(
                    "test.globalSetup",
                    ConfigValueKind::String("./test/global-setup.ts".to_string()),
                ),
            ],
        );
        let adapter = VitestAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.test_patterns, vec!["**/*.{test,spec}.ts"]);
        assert_eq!(inputs.setup_files, vec![PathBuf::from("./test/setup.ts")]);
        assert_eq!(
            inputs.global_setup_files,
            vec![PathBuf::from("./test/global-setup.ts")]
        );
        assert_eq!(inputs.framework, Some("vitest".to_string()));
    }

    #[test]
    fn vitest_adapter_matches_workspace() {
        let config = make_config("vitest.workspace.ts", vec![]);
        assert!(VitestAdapter.matches(&config));
    }

    #[test]
    fn vitest_adapter_extracts_aliases() {
        let config = make_config(
            "vitest.config.ts",
            vec![cv(
                "resolve.alias.@",
                ConfigValueKind::String("./src".to_string()),
            )],
        );
        let inputs = VitestAdapter.extract(&config);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@");
        assert_eq!(inputs.aliases[0].target, "./src");
    }

    #[test]
    fn nuxt_adapter_extracts_aliases_and_dirs() {
        let config = make_config(
            "nuxt.config.ts",
            vec![
                cv(
                    "alias.@",
                    ConfigValueKind::String("./src".to_string()),
                ),
                cv(
                    "imports.dirs",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "composables".to_string(),
                    )]),
                ),
                cv(
                    "components.dirs",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "components".to_string(),
                    )]),
                ),
                cv(
                    "dir.pages",
                    ConfigValueKind::String("pages".to_string()),
                ),
                cv(
                    "modules",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "@nuxtjs/tailwindcss".to_string(),
                    )]),
                ),
            ],
        );
        let adapter = NuxtAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@");
        assert_eq!(
            inputs.auto_import_roots,
            vec![PathBuf::from("composables"), PathBuf::from("components")]
        );
        assert_eq!(
            inputs.production_entrypoints,
            vec![PathBuf::from("pages")]
        );
        assert_eq!(inputs.externals, vec!["@nuxtjs/tailwindcss"]);
    }

    #[test]
    fn astro_adapter_extracts_src_and_out() {
        let config = make_config(
            "astro.config.mjs",
            vec![
                cv("srcDir", ConfigValueKind::String("./src".to_string())),
                cv("outDir", ConfigValueKind::String("./dist".to_string())),
                cv(
                    "integrations",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "@astrojs/react".to_string(),
                    )]),
                ),
                cv(
                    "vite.resolve.alias.@components",
                    ConfigValueKind::String("./src/components".to_string()),
                ),
            ],
        );
        let adapter = AstroAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("./src")]);
        assert_eq!(inputs.ignore_patterns, vec!["./dist"]);
        assert_eq!(inputs.externals, vec!["@astrojs/react"]);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@components");
    }

    #[test]
    fn sveltekit_adapter_extracts_files_and_alias() {
        let config = make_config(
            "svelte.config.js",
            vec![
                cv(
                    "kit.files.routes",
                    ConfigValueKind::String("src/routes".to_string()),
                ),
                cv(
                    "kit.files.lib",
                    ConfigValueKind::String("src/lib".to_string()),
                ),
                cv(
                    "kit.alias.$lib",
                    ConfigValueKind::String("./src/lib".to_string()),
                ),
            ],
        );
        let adapter = SvelteKitAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(
            inputs.source_roots,
            vec![PathBuf::from("src/routes"), PathBuf::from("src/lib")]
        );
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "$lib");
        assert_eq!(inputs.aliases[0].target, "./src/lib");
        assert_eq!(inputs.framework, Some("sveltekit".to_string()));
    }

    #[test]
    fn remix_adapter_extracts_app_dir_and_routes() {
        let config = make_config(
            "remix.config.js",
            vec![
                cv(
                    "appDirectory",
                    ConfigValueKind::String("app".to_string()),
                ),
                cv(
                    "serverBuildPath",
                    ConfigValueKind::String("build/index.js".to_string()),
                ),
            ],
        );
        let adapter = RemixAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("app")]);
        assert_eq!(inputs.ignore_patterns, vec!["build/index.js"]);
    }

    #[test]
    fn angular_adapter_extracts_build_options() {
        let config = make_config(
            "angular.json",
            vec![
                cv(
                    "projects.my-app.architect.build.options.main",
                    ConfigValueKind::String("src/main.ts".to_string()),
                ),
                cv(
                    "projects.my-app.architect.build.options.styles",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "src/styles.css".to_string(),
                    )]),
                ),
                cv(
                    "projects.my-app.architect.test.options.main",
                    ConfigValueKind::String("src/test.ts".to_string()),
                ),
            ],
        );
        let adapter = AngularAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert!(inputs.entrypoints.contains(&PathBuf::from("src/main.ts")));
        assert!(inputs.entrypoints.contains(&PathBuf::from("src/styles.css")));
        assert_eq!(inputs.setup_files, vec![PathBuf::from("src/test.ts")]);
    }

    #[test]
    fn angular_adapter_matches_angular_config() {
        let config = make_config("angular.config.js", vec![]);
        assert!(AngularAdapter.matches(&config));
    }

    #[test]
    fn nx_adapter_extracts_build_main_and_deps() {
        let config = make_config(
            "project.json",
            vec![
                cv(
                    "targets.build.options.main",
                    ConfigValueKind::String("src/main.ts".to_string()),
                ),
                cv(
                    "implicitDependencies",
                    ConfigValueKind::Array(vec![ConfigValueKind::String("shared-lib".to_string())]),
                ),
            ],
        );
        let adapter = NxAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.entrypoints, vec![PathBuf::from("src/main.ts")]);
        assert_eq!(inputs.externals, vec!["shared-lib"]);
    }

    #[test]
    fn nx_adapter_matches_nx_json() {
        let config = make_config("nx.json", vec![]);
        assert!(NxAdapter.matches(&config));
    }

    #[test]
    fn turborepo_adapter_sets_medium_confidence() {
        let config = make_config("turbo.json", vec![]);
        let adapter = TurborepoAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.confidence, AdapterConfidence::Medium);
        assert_eq!(inputs.framework, Some("turborepo".to_string()));
    }

    #[test]
    fn vitepress_adapter_extracts_src_and_out() {
        let config = make_config(
            "vitepress.config.ts",
            vec![
                cv("srcDir", ConfigValueKind::String("docs".to_string())),
                cv("outDir", ConfigValueKind::String(".vitepress/dist".to_string())),
            ],
        );
        let adapter = VitePressAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("docs")]);
        assert_eq!(inputs.ignore_patterns, vec![".vitepress/dist"]);
    }

    #[test]
    fn vitepress_adapter_matches_dot_vitepress_config() {
        let config = make_config(".vitepress/config.ts", vec![]);
        assert!(VitePressAdapter.matches(&config));
    }

    #[test]
    fn docusaurus_adapter_extracts_presets_and_plugins() {
        let config = make_config(
            "docusaurus.config.js",
            vec![
                cv(
                    "presets",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "@docusaurus/preset-classic".to_string(),
                    )]),
                ),
                cv(
                    "plugins",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "@docusaurus/plugin-content-blog".to_string(),
                    )]),
                ),
            ],
        );
        let adapter = DocusaurusAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(
            inputs.externals,
            vec![
                "@docusaurus/preset-classic",
                "@docusaurus/plugin-content-blog"
            ]
        );
    }

    #[test]
    fn rollup_adapter_extracts_input_and_external() {
        let config = make_config(
            "rollup.config.js",
            vec![
                cv(
                    "input",
                    ConfigValueKind::String("src/index.js".to_string()),
                ),
                cv(
                    "external",
                    ConfigValueKind::Array(vec![
                        ConfigValueKind::String("lodash".to_string()),
                        ConfigValueKind::String("react".to_string()),
                    ]),
                ),
            ],
        );
        let adapter = RollupAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.entrypoints, vec![PathBuf::from("src/index.js")]);
        assert_eq!(inputs.externals, vec!["lodash", "react"]);
    }

    #[test]
    fn rollup_adapter_extracts_object_input() {
        let config = make_config(
            "rollup.config.mjs",
            vec![cv(
                "input",
                ConfigValueKind::Object(vec![
                    (
                        "main".to_string(),
                        ConfigValueKind::String("src/main.js".to_string()),
                    ),
                    (
                        "utils".to_string(),
                        ConfigValueKind::String("src/utils.js".to_string()),
                    ),
                ]),
            )],
        );
        let inputs = RollupAdapter.extract(&config);
        assert_eq!(inputs.entrypoints.len(), 2);
    }

    #[test]
    fn rspack_adapter_extracts_entry_alias_externals() {
        let config = make_config(
            "rspack.config.js",
            vec![
                cv(
                    "entry",
                    ConfigValueKind::String("./src/index.js".to_string()),
                ),
                cv(
                    "resolve.alias.@",
                    ConfigValueKind::String("./src".to_string()),
                ),
                cv(
                    "externals",
                    ConfigValueKind::Array(vec![ConfigValueKind::String("jquery".to_string())]),
                ),
            ],
        );
        let adapter = RspackAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.entrypoints, vec![PathBuf::from("./src/index.js")]);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@");
        assert_eq!(inputs.externals, vec!["jquery"]);
    }

    #[test]
    fn rsbuild_adapter_extracts_entry_and_alias() {
        let config = make_config(
            "rsbuild.config.ts",
            vec![
                cv(
                    "source.entry.main",
                    ConfigValueKind::String("./src/index.tsx".to_string()),
                ),
                cv(
                    "source.alias.@",
                    ConfigValueKind::String("./src".to_string()),
                ),
            ],
        );
        let adapter = RsbuildAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.entrypoints, vec![PathBuf::from("./src/index.tsx")]);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "@");
    }

    #[test]
    fn parcel_adapter_sets_low_confidence() {
        let config = make_config(".parcelrc", vec![]);
        let adapter = ParcelAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.confidence, AdapterConfidence::Low);
        assert_eq!(inputs.framework, Some("parcel".to_string()));
    }

    #[test]
    fn gatsby_adapter_extracts_plugins() {
        let config = make_config(
            "gatsby-config.js",
            vec![cv(
                "plugins",
                ConfigValueKind::Array(vec![
                    ConfigValueKind::String("gatsby-plugin-react-helmet".to_string()),
                    ConfigValueKind::String("gatsby-plugin-mdx".to_string()),
                ]),
            )],
        );
        let adapter = GatsbyAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(
            inputs.externals,
            vec!["gatsby-plugin-react-helmet", "gatsby-plugin-mdx"]
        );
    }

    #[test]
    fn gatsby_adapter_matches_gatsby_node() {
        let config = make_config("gatsby-node.js", vec![]);
        assert!(GatsbyAdapter.matches(&config));
    }

    #[test]
    fn nitro_adapter_extracts_alias_imports_externals() {
        let config = make_config(
            "nitro.config.ts",
            vec![
                cv(
                    "alias.#internal",
                    ConfigValueKind::String("./server/internal".to_string()),
                ),
                cv(
                    "imports.dirs",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "server/utils".to_string(),
                    )]),
                ),
                cv(
                    "externals",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "better-sqlite3".to_string(),
                    )]),
                ),
            ],
        );
        let adapter = NitroAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.aliases.len(), 1);
        assert_eq!(inputs.aliases[0].pattern, "#internal");
        assert_eq!(
            inputs.auto_import_roots,
            vec![PathBuf::from("server/utils")]
        );
        assert_eq!(inputs.externals, vec!["better-sqlite3"]);
    }

    #[test]
    fn react_router_adapter_extracts_app_dir_and_routes() {
        let config = make_config(
            "react-router.config.ts",
            vec![
                cv(
                    "appDirectory",
                    ConfigValueKind::String("app".to_string()),
                ),
                cv(
                    "serverBuildPath",
                    ConfigValueKind::String("build/server.js".to_string()),
                ),
            ],
        );
        let adapter = ReactRouterAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("app")]);
        assert_eq!(inputs.ignore_patterns, vec!["build/server.js"]);
        assert_eq!(inputs.framework, Some("react-router".to_string()));
    }

    #[test]
    fn qwik_adapter_extracts_srcdir_with_medium_confidence() {
        let config = make_config(
            "qwik.config.ts",
            vec![cv(
                "srcDir",
                ConfigValueKind::String("./src".to_string()),
            )],
        );
        let adapter = QwikAdapter;
        assert!(adapter.matches(&config));
        let inputs = adapter.extract(&config);
        assert_eq!(inputs.source_roots, vec![PathBuf::from("./src")]);
        assert_eq!(inputs.confidence, AdapterConfidence::Medium);
    }

    #[test]
    fn new_adapters_do_not_match_wrong_config() {
        let config = make_config("tsconfig.json", vec![]);
        assert!(!VitestAdapter.matches(&config));
        assert!(!NuxtAdapter.matches(&config));
        assert!(!AstroAdapter.matches(&config));
        assert!(!SvelteKitAdapter.matches(&config));
        assert!(!RemixAdapter.matches(&config));
        assert!(!AngularAdapter.matches(&config));
        assert!(!NxAdapter.matches(&config));
        assert!(!TurborepoAdapter.matches(&config));
        assert!(!VitePressAdapter.matches(&config));
        assert!(!DocusaurusAdapter.matches(&config));
        assert!(!RollupAdapter.matches(&config));
        assert!(!RspackAdapter.matches(&config));
        assert!(!RsbuildAdapter.matches(&config));
        assert!(!ParcelAdapter.matches(&config));
        assert!(!GatsbyAdapter.matches(&config));
        assert!(!NitroAdapter.matches(&config));
        assert!(!ReactRouterAdapter.matches(&config));
        assert!(!QwikAdapter.matches(&config));
    }

    #[test]
    fn merge_extends_new_fields() {
        let mut a = ConfigInputs {
            runtime_entrypoints: vec![PathBuf::from("a.ts")],
            setup_files: vec![PathBuf::from("setup-a.ts")],
            ..Default::default()
        };
        let b = ConfigInputs {
            runtime_entrypoints: vec![PathBuf::from("b.ts")],
            production_entrypoints: vec![PathBuf::from("prod.ts")],
            development_entrypoints: vec![PathBuf::from("dev.ts")],
            story_entrypoints: vec![PathBuf::from("story.ts")],
            setup_files: vec![PathBuf::from("setup-b.ts")],
            global_setup_files: vec![PathBuf::from("global.ts")],
            auto_import_roots: vec![PathBuf::from("composables")],
            confidence: AdapterConfidence::Low,
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(
            a.runtime_entrypoints,
            vec![PathBuf::from("a.ts"), PathBuf::from("b.ts")]
        );
        assert_eq!(a.production_entrypoints, vec![PathBuf::from("prod.ts")]);
        assert_eq!(a.development_entrypoints, vec![PathBuf::from("dev.ts")]);
        assert_eq!(a.story_entrypoints, vec![PathBuf::from("story.ts")]);
        assert_eq!(
            a.setup_files,
            vec![PathBuf::from("setup-a.ts"), PathBuf::from("setup-b.ts")]
        );
        assert_eq!(a.global_setup_files, vec![PathBuf::from("global.ts")]);
        assert_eq!(a.auto_import_roots, vec![PathBuf::from("composables")]);
        // First non-default confidence is kept
        assert_eq!(a.confidence, AdapterConfidence::Low);
    }

    #[test]
    fn merge_keeps_first_non_default_confidence() {
        let mut a = ConfigInputs {
            confidence: AdapterConfidence::Medium,
            ..Default::default()
        };
        let b = ConfigInputs {
            confidence: AdapterConfidence::Low,
            ..Default::default()
        };
        a.merge(b);
        // Already non-default, so it stays Medium.
        assert_eq!(a.confidence, AdapterConfidence::Medium);
    }

    #[test]
    fn extract_all_inputs_includes_new_adapters() {
        let configs = vec![
            make_config(
                "nuxt.config.ts",
                vec![cv(
                    "modules",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "@nuxtjs/tailwindcss".to_string(),
                    )]),
                )],
            ),
            make_config(
                "vitest.config.ts",
                vec![cv(
                    "test.include",
                    ConfigValueKind::Array(vec![ConfigValueKind::String(
                        "**/*.spec.ts".to_string(),
                    )]),
                )],
            ),
        ];
        let merged = extract_all_inputs(&configs);
        assert_eq!(merged.externals, vec!["@nuxtjs/tailwindcss"]);
        // vitest.config matches both ViteAdapter and VitestAdapter
        assert!(merged.test_patterns.contains(&"**/*.spec.ts".to_string()));
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
