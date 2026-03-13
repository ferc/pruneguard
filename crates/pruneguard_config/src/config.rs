use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::merge::Merge;

/// Top-level configuration for pruneguard.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PruneguardConfig {
    /// JSON Schema reference.
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    /// Configs to extend from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Vec<String>>,

    /// Glob patterns for files to ignore entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_patterns: Vec<String>,

    /// Workspace configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<WorkspacesConfig>,

    /// Module resolver configuration.
    #[serde(default)]
    pub resolver: ResolverConfig,

    /// Entrypoint configuration.
    #[serde(default)]
    pub entrypoints: EntrypointsConfig,

    /// Analysis severity levels.
    #[serde(default)]
    pub analysis: AnalysisConfig,

    /// Semantic precision layer configuration.
    #[serde(default)]
    pub semantic: SemanticConfig,

    /// Custom rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<RulesConfig>,

    /// Code ownership configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership: Option<OwnershipConfig>,

    /// Framework detection packs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frameworks: Option<FrameworksConfig>,

    /// Per-path or per-workspace overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<OverrideConfig>,

    /// Suppress specific finding kinds from the report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_issues: Vec<IgnoreIssueRule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacesConfig {
    /// Workspace root globs (e.g. `["apps/*", "packages/*"]`).
    #[serde(default)]
    pub roots: Vec<String>,

    /// Package manager to use for workspace discovery.
    #[serde(default)]
    pub package_manager: PackageManager,

    /// Include only workspaces matching these patterns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,

    /// Exclude workspaces matching these patterns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    #[default]
    Auto,
    Pnpm,
    Npm,
    Yarn,
    Bun,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct ResolverConfig {
    /// Paths to tsconfig files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tsconfig: Vec<String>,

    /// Export conditions to use when resolving package exports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<String>,

    /// File extensions to resolve.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,

    /// Whether to respect the `exports` field in package.json.
    #[serde(default = "default_true")]
    pub respect_exports: bool,

    /// Whether to preserve symlinks during resolution.
    #[serde(default)]
    pub preserve_symlinks: bool,

    // Non-standard edge detection toggles.
    /// Whether to detect `require.resolve()` calls.
    #[serde(default = "default_true")]
    pub detect_require_resolve: bool,
    /// Whether to detect `import.meta.resolve()` calls.
    #[serde(default = "default_true")]
    pub detect_import_meta_resolve: bool,
    /// Whether to detect `import.meta.glob()` calls.
    #[serde(default = "default_true")]
    pub detect_import_meta_glob: bool,
    /// Whether to detect `require.context()` calls.
    #[serde(default = "default_true")]
    pub detect_require_context: bool,
    /// Whether to detect `new URL()` constructor patterns.
    #[serde(default = "default_true")]
    pub detect_url_constructor: bool,
    /// Whether to detect `JSDoc` `@import` tags.
    #[serde(default = "default_true")]
    pub detect_jsdoc_imports: bool,
    /// Whether to detect triple-slash reference directives.
    #[serde(default = "default_true")]
    pub detect_triple_slash: bool,
    /// Whether to detect TypeScript `import =` statements.
    #[serde(default = "default_true")]
    pub detect_import_equals: bool,
    /// Whether to detect type-only imports.
    #[serde(default = "default_true")]
    pub detect_type_imports: bool,
    /// Whether to detect webpack alias patterns.
    #[serde(default = "default_true")]
    pub detect_webpack_aliases: bool,
    /// Whether to detect Babel alias patterns.
    #[serde(default = "default_true")]
    pub detect_babel_aliases: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            tsconfig: Vec::new(),
            conditions: Vec::new(),
            extensions: Vec::new(),
            respect_exports: true,
            preserve_symlinks: false,
            detect_require_resolve: true,
            detect_import_meta_resolve: true,
            detect_import_meta_glob: true,
            detect_require_context: true,
            detect_url_constructor: true,
            detect_jsdoc_imports: true,
            detect_triple_slash: true,
            detect_import_equals: true,
            detect_type_imports: true,
            detect_webpack_aliases: true,
            detect_babel_aliases: true,
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntrypointsConfig {
    /// Auto-detect entrypoints from package.json and conventions.
    #[serde(default = "default_true")]
    pub auto: bool,

    /// Additional entrypoint globs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,

    /// Exclude entrypoints matching these globs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,

    /// Whether to include test files as entrypoints.
    #[serde(default)]
    pub include_tests: bool,

    /// Whether to include story files as entrypoints.
    #[serde(default)]
    pub include_stories: bool,

    /// When true, runtime entrypoint exports are eligible to be reported as
    /// unused.  By default (`false`), only public-API entrypoint exports are
    /// checked.
    #[serde(default)]
    pub include_entry_exports: bool,

    /// Profile-specific entrypoint overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiles: Option<ProfilesConfig>,
}

impl Default for EntrypointsConfig {
    fn default() -> Self {
        Self {
            auto: true,
            include: Vec::new(),
            exclude: Vec::new(),
            include_tests: false,
            include_stories: false,
            include_entry_exports: false,
            profiles: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProfilesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub development: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<Vec<String>>,
}

/// Severity level for analysis features.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisSeverity {
    Off,
    Info,
    #[default]
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisConfig {
    #[serde(default)]
    pub unused_exports: AnalysisSeverity,
    #[serde(default)]
    pub unused_files: AnalysisSeverity,
    #[serde(default)]
    pub unused_packages: AnalysisSeverity,
    #[serde(default)]
    pub unused_dependencies: AnalysisSeverity,
    #[serde(default)]
    pub cycles: AnalysisSeverity,
    #[serde(default)]
    pub boundaries: AnalysisSeverity,
    #[serde(default)]
    pub ownership: AnalysisSeverity,
    #[serde(default)]
    pub impact: AnalysisSeverity,

    /// Report unused exported class/enum members (methods, properties, variants).
    #[serde(default)]
    pub unused_members: AnalysisSeverity,

    /// Report duplicate exports (same symbol re-exported from multiple paths).
    #[serde(default)]
    pub duplicate_exports: AnalysisSeverity,

    /// When true, exports consumed only within the same file are still
    /// reported as unused.
    #[serde(default)]
    pub ignore_exports_used_in_file: bool,

    /// Glob patterns for member names to ignore in unused-member analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_members: Vec<String>,

    /// `JSDoc` tag names that mark a member as public/intentionally exported.
    /// When empty, `@public` is used as the default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_tag_names: Vec<String>,

    /// Whether write-only member references count as "used".
    /// When true (default), a member that is only written to (e.g. `obj.field = x`)
    /// but never read is reported as unused.
    #[serde(default = "default_true")]
    pub member_write_only_is_unused: bool,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            unused_exports: AnalysisSeverity::default(),
            unused_files: AnalysisSeverity::default(),
            unused_packages: AnalysisSeverity::default(),
            unused_dependencies: AnalysisSeverity::default(),
            cycles: AnalysisSeverity::default(),
            boundaries: AnalysisSeverity::default(),
            ownership: AnalysisSeverity::default(),
            impact: AnalysisSeverity::default(),
            unused_members: AnalysisSeverity::default(),
            duplicate_exports: AnalysisSeverity::default(),
            ignore_exports_used_in_file: false,
            ignore_members: Vec::new(),
            public_tag_names: Vec::new(),
            member_write_only_is_unused: true,
        }
    }
}

/// Mode for the optional semantic precision helper.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SemanticMode {
    Off,
    #[default]
    Auto,
    Required,
}

/// Configuration for the optional semantic precision layer.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SemanticConfig {
    /// Whether to use the semantic helper: off, auto (default), or required.
    #[serde(default)]
    pub mode: SemanticMode,

    /// Maximum percentage of cold-scan overhead the helper is allowed to add (0-100).
    #[serde(default = "default_max_cold_overhead_pct")]
    pub max_cold_overhead_pct: u8,

    /// Maximum files to include in a single query batch to the helper.
    #[serde(default = "default_max_files_per_query_batch")]
    pub max_files_per_query_batch: usize,

    /// Maximum number of TypeScript project references to traverse.
    #[serde(default = "default_max_project_refs")]
    pub max_project_refs: usize,

    /// Maximum wall-clock milliseconds the helper is allowed to run.
    #[serde(default = "default_max_helper_wall_ms")]
    pub max_helper_wall_ms: u64,

    /// Minimum uncertainty score (0-100) a candidate must have before being
    /// sent to the semantic helper. Lower values mean more candidates are refined.
    #[serde(default = "default_min_uncertainty_score")]
    pub min_uncertainty_score: u8,
}

const fn default_max_cold_overhead_pct() -> u8 {
    20
}
const fn default_max_files_per_query_batch() -> usize {
    128
}
const fn default_max_project_refs() -> usize {
    8
}
const fn default_max_helper_wall_ms() -> u64 {
    1200
}
const fn default_min_uncertainty_score() -> u8 {
    60
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            mode: SemanticMode::default(),
            max_cold_overhead_pct: default_max_cold_overhead_pct(),
            max_files_per_query_batch: default_max_files_per_query_batch(),
            max_project_refs: default_max_project_refs(),
            max_helper_wall_ms: default_max_helper_wall_ms(),
            min_uncertainty_score: default_min_uncertainty_score(),
        }
    }
}

/// Rule for suppressing specific finding kinds.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreIssueRule {
    /// The finding kind to suppress (e.g. "unusedExport", "unusedFile", "cycle").
    pub kind: String,
    /// Optional glob patterns — only suppress in matching files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Optional comment explaining why this is suppressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Workspace names to scope this rule to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    /// Package names to scope this rule to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<String>,
    /// Symbol names to scope this rule to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    /// Parent symbol names to scope this rule to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_symbols: Vec<String>,
    /// Additional finding codes to match (beyond the primary `kind`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RulesConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub name: String,
    #[serde(default)]
    pub severity: AnalysisSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<RuleFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<RuleFilter>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_not: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_not: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_not: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_not: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reachable_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaches: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entrypoint_kinds: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipConfig {
    #[serde(default)]
    pub import_codeowners: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teams: Option<rustc_hash::FxHashMap<String, TeamConfig>>,
    #[serde(default)]
    pub unowned_severity: AnalysisSeverity,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TeamConfig {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FrameworkToggle {
    Off,
    Auto,
    On,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct FrameworksConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vite: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vitest: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jest: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storybook: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nuxt: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub astro: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sveltekit: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remix: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angular: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nx: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turborepo: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playwright: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cypress: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vitepress: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docusaurus: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vue: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svelte: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub babel: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tanstack_router: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vike: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rslib: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playwright_ct: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playwright_test: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nitro: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub react_router: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsbuild: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parcel: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qwik: Option<FrameworkToggle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_dev: Option<FrameworkToggle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct OverrideConfig {
    /// File globs this override applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Workspace names this override applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    /// Deterministic tags applied to matching files/workspaces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Analysis overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisConfig>,
    /// Entrypoint overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoints: Option<EntrypointsConfig>,
    /// Additional ignore patterns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_patterns: Vec<String>,
}

const fn default_true() -> bool {
    true
}

/// Config file search names in priority order.
const CONFIG_FILES: &[&str] = &["pruneguard.json", ".pruneguardrc.json"];

impl PruneguardConfig {
    /// Load config from the given directory, searching for config files.
    /// If `explicit_path` is provided, only that file is loaded.
    pub fn load(cwd: &Path, explicit_path: Option<&Path>) -> Result<Self, ConfigError> {
        let path = if let Some(explicit) = explicit_path {
            if explicit.is_absolute() { explicit.to_path_buf() } else { cwd.join(explicit) }
        } else {
            Self::find_config(cwd)?
        };

        let content = std::fs::read_to_string(&path).map_err(|source| ConfigError::ReadError {
            path: path.display().to_string(),
            source,
        })?;

        let mut config: Self = serde_json::from_str(&content).map_err(|source| {
            ConfigError::ParseError { path: path.display().to_string(), source }
        })?;

        // Resolve extends
        if let Some(extends) = config.extends.take() {
            let base_dir = path.parent().unwrap_or(cwd);
            for extend_path in &extends {
                let extend_file = base_dir.join(extend_path);
                let base = Self::load(cwd, Some(&extend_file))?;
                config.merge_from(&base);
            }
        }

        Ok(config)
    }

    /// Search for a config file in the given directory.
    fn find_config(cwd: &Path) -> Result<PathBuf, ConfigError> {
        for name in CONFIG_FILES {
            let candidate = cwd.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        Err(ConfigError::NotFound)
    }

    /// Generate a minimal config file in the current directory.
    ///
    /// Most repos work well with zero configuration. This generates a minimal
    /// `pruneguard.json` with just the `$schema` field for editor autocomplete.
    pub fn init() -> Result<(), ConfigError> {
        let config = Self {
            schema: Some("./node_modules/pruneguard/configuration_schema.json".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&config).expect("failed to serialize");
        std::fs::write("pruneguard.json", json)?;
        Ok(())
    }

    /// Generate the JSON Schema for the configuration.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}
