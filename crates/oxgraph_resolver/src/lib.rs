use std::path::{Path, PathBuf};

use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use oxgraph_config::ResolverConfig;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Result of resolving a module specifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedModule {
    /// The resolved file path.
    pub path: PathBuf,
    /// Whether this was resolved via package exports.
    pub via_exports: bool,
}

/// Graph-facing kind of a resolved dependency edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedEdgeKind {
    StaticImportValue,
    StaticImportType,
    DynamicImport,
    Require,
    SideEffectImport,
    ReExportNamed,
    ReExportAll,
}

/// A resolved edge emitted while extracting a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedEdge {
    pub from: PathBuf,
    pub specifier: String,
    pub to_file: Option<PathBuf>,
    pub to_dependency: Option<String>,
    pub kind: ResolvedEdgeKind,
    pub outcome: ResolutionOutcome,
    pub unresolved_reason: Option<UnresolvedReason>,
    pub via_exports: bool,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionOutcome {
    ResolvedToFile,
    ResolvedToDependency,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnresolvedReason {
    MissingFile,
    UnsupportedSpecifier,
    TsconfigPathMiss,
    ExportsConditionMiss,
    Externalized,
}

impl UnresolvedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingFile => "missing-file",
            Self::UnsupportedSpecifier => "unsupported-specifier",
            Self::TsconfigPathMiss => "tsconfig-path-miss",
            Self::ExportsConditionMiss => "exports-condition-miss",
            Self::Externalized => "externalized",
        }
    }
}

/// Module resolver built on `oxc_resolver`.
pub struct ModuleResolver {
    inner: Resolver,
}

impl ModuleResolver {
    /// Create a new resolver from config.
    pub fn new(config: &ResolverConfig, cwd: &Path) -> Self {
        let mut options = ResolveOptions { cwd: Some(cwd.to_path_buf()), ..ResolveOptions::default() };

        if config.extensions.is_empty() {
            options.extensions = vec![
                ".ts".to_string(),
                ".tsx".to_string(),
                ".js".to_string(),
                ".jsx".to_string(),
                ".mjs".to_string(),
                ".cjs".to_string(),
                ".mts".to_string(),
                ".cts".to_string(),
                ".json".to_string(),
            ];
        } else {
            options.extensions = config
                .extensions
                .iter()
                .map(|ext| if ext.starts_with('.') { ext.clone() } else { format!(".{ext}") })
                .collect();
        }

        if !config.conditions.is_empty() {
            options.condition_names.clone_from(&config.conditions);
        }

        options.symlinks = !config.preserve_symlinks;
        options.tsconfig = Some(if let Some(tsconfig) = config.tsconfig.first() {
            TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: if Path::new(tsconfig).is_absolute() {
                    PathBuf::from(tsconfig)
                } else {
                    cwd.join(tsconfig)
                },
                references: TsconfigReferences::Auto,
            })
        } else {
            TsconfigDiscovery::Auto
        });

        let inner = Resolver::new(options);
        Self { inner }
    }

    /// Resolve a module specifier from a given file.
    pub fn resolve(&self, specifier: &str, from: &Path) -> Result<ResolvedModule, ResolveError> {
        let directory = from.parent().unwrap_or(from);
        match self.inner.resolve(directory, specifier) {
            Ok(resolution) => Ok(ResolvedModule {
                path: resolution.into_path_buf(),
                via_exports: false, // TODO: detect this
            }),
            Err(err) => Err(ResolveError::NotFound {
                specifier: specifier.to_string(),
                from: from.to_path_buf(),
                reason: Some(classify_unresolved_reason(specifier, &err.to_string())),
            }),
        }
    }
}

/// Debug resolve a specifier from a file and return a human-readable result.
pub fn debug_resolve(cwd: &Path, config: &ResolverConfig, specifier: &str, from: &Path) -> String {
    let resolver = ModuleResolver::new(config, cwd);
    match resolver.resolve(specifier, from) {
        Ok(module) => format!(
            "{specifier} -> {}{}",
            module.path.display(),
            if module.via_exports { " (via exports)" } else { "" }
        ),
        Err(err) => format!(
            "{specifier} -> UNRESOLVED ({})",
            err.reason()
                .map_or_else(|| err.to_string(), |reason| reason.as_str().to_string())
        ),
    }
}

/// Infer the dependency package name from a bare module specifier.
pub fn dependency_name(specifier: &str) -> Option<String> {
    if specifier.starts_with('.') || specifier.starts_with('/') {
        return None;
    }

    let mut parts = specifier.split('/');
    let first = parts.next()?;
    if first.starts_with('@') {
        let second = parts.next()?;
        Some(format!("{first}/{second}"))
    } else {
        Some(first.to_string())
    }
}

/// Cache of resolved modules to avoid redundant resolution.
#[derive(Debug, Default)]
pub struct ResolutionCache {
    cache: FxHashMap<(PathBuf, String), Option<PathBuf>>,
}

impl ResolutionCache {
    pub fn get(&self, from: &Path, specifier: &str) -> Option<&Option<PathBuf>> {
        self.cache.get(&(from.to_path_buf(), specifier.to_string()))
    }

    pub fn insert(&mut self, from: PathBuf, specifier: String, resolved: Option<PathBuf>) {
        self.cache.insert((from, specifier), resolved);
    }
}

/// Errors from module resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("cannot resolve '{specifier}' from {from}")]
    NotFound {
        specifier: String,
        from: PathBuf,
        reason: Option<UnresolvedReason>,
    },
}

impl ResolveError {
    pub const fn reason(&self) -> Option<UnresolvedReason> {
        match self {
            Self::NotFound { reason, .. } => *reason,
        }
    }
}

fn classify_unresolved_reason(specifier: &str, error: &str) -> UnresolvedReason {
    if specifier.starts_with("node:") {
        return UnresolvedReason::Externalized;
    }

    if !specifier.starts_with('.') && !specifier.starts_with('/') {
        if error.contains("tsconfig") || error.contains("paths") {
            return UnresolvedReason::TsconfigPathMiss;
        }
        if error.contains("exports") || error.contains("condition") {
            return UnresolvedReason::ExportsConditionMiss;
        }
        return UnresolvedReason::MissingFile;
    }

    if specifier.contains('*') || specifier.contains('?') {
        return UnresolvedReason::UnsupportedSpecifier;
    }

    UnresolvedReason::MissingFile
}
