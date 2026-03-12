use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::merge::Merge;

/// Top-level configuration for oxgraph.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OxgraphConfig {
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
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            tsconfig: Vec::new(),
            conditions: Vec::new(),
            extensions: Vec::new(),
            respect_exports: true,
            preserve_symlinks: false,
        }
    }
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct OverrideConfig {
    /// File globs this override applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Workspace names this override applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
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
const CONFIG_FILES: &[&str] = &["oxgraph.json", ".oxgraphrc.json"];

impl OxgraphConfig {
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

    /// Generate a default config file in the current directory.
    pub fn init() -> Result<(), ConfigError> {
        let config = Self {
            schema: Some("./node_modules/oxgraph/configuration_schema.json".to_string()),
            workspaces: Some(WorkspacesConfig {
                package_manager: PackageManager::Auto,
                roots: vec!["apps/*".to_string(), "packages/*".to_string()],
                ..Default::default()
            }),
            entrypoints: EntrypointsConfig { auto: true, ..Default::default() },
            analysis: AnalysisConfig {
                unused_exports: AnalysisSeverity::Warn,
                unused_files: AnalysisSeverity::Warn,
                unused_dependencies: AnalysisSeverity::Warn,
                ..Default::default()
            },
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&config).expect("failed to serialize");
        std::fs::write("oxgraph.json", json)?;
        Ok(())
    }

    /// Generate the JSON Schema for the configuration.
    pub fn json_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Self)
    }
}
