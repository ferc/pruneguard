use std::path::{Path, PathBuf};

use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use pruneguard_config::ResolverConfig;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Bump this whenever hardcoded resolver behaviour changes (e.g. `extension_alias`,
/// default extensions, condition names) so that the analysis cache is invalidated.
pub const RESOLVER_LOGIC_VERSION: u32 = 4;

/// Result of resolving a module specifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedModule {
    /// The resolved file path.
    pub path: PathBuf,
    /// Whether this was resolved via package exports.
    pub via_exports: bool,
    /// The subpath pattern that matched in package.json exports, if any.
    /// E.g. for `@scope/pkg/utils`, this might be `./utils` or `./*`.
    pub exports_subpath: Option<String>,
    /// Which condition branch was selected (e.g. "import", "require", "types", "default").
    pub exports_condition: Option<String>,
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
    /// The subpath pattern matched in package.json exports (e.g. `./utils`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exports_subpath: Option<String>,
    /// The condition branch that was selected (e.g. "import", "types", "default").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exports_condition: Option<String>,
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
    /// Map of workspace package names to their root directories.
    /// Enables resolution of deep subpath imports like `@scope/pkg/path/to/file`.
    workspace_roots: FxHashMap<String, PathBuf>,
}

impl ModuleResolver {
    /// Create a new resolver from config.
    pub fn new(config: &ResolverConfig, cwd: &Path) -> Self {
        let mut options =
            ResolveOptions { cwd: Some(cwd.to_path_buf()), ..ResolveOptions::default() };

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

        // Map .js/.jsx/.mjs/.cjs imports to their TypeScript equivalents.
        // This is required for moduleResolution "bundler"/"node16"/"nodenext"
        // where TypeScript source files use .js extensions in import specifiers.
        options.extension_alias = vec![
            (".js".to_string(), vec![".ts".into(), ".tsx".into(), ".js".into()]),
            (".jsx".to_string(), vec![".tsx".into(), ".jsx".into()]),
            (".mjs".to_string(), vec![".mts".into(), ".mjs".into()]),
            (".cjs".to_string(), vec![".cts".into(), ".cjs".into()]),
        ];

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
        Self { inner, workspace_roots: FxHashMap::default() }
    }

    /// Register workspace package name → root directory mappings.
    /// This enables resolution of deep subpath imports into workspace packages
    /// (e.g. `@calcom/features/auth/lib/getLocale` → `packages/features/auth/lib/getLocale.ts`).
    pub fn set_workspace_roots(&mut self, roots: FxHashMap<String, PathBuf>) {
        self.workspace_roots = roots;
    }

    /// Resolve a module specifier from a given file.
    pub fn resolve(&self, specifier: &str, from: &Path) -> Result<ResolvedModule, ResolveError> {
        let directory = from.parent().unwrap_or(from);
        match self.inner.resolve(directory, specifier) {
            Ok(resolution) => {
                let via_exports = resolved_via_package_exports(specifier, &resolution);
                let (exports_subpath, exports_condition) = if via_exports {
                    extract_exports_attribution(specifier, &resolution)
                } else {
                    (None, None)
                };
                Ok(ResolvedModule {
                    path: resolution.into_path_buf(),
                    via_exports,
                    exports_subpath,
                    exports_condition,
                })
            }
            Err(err) => {
                // Try workspace-aware resolution: map `@scope/pkg/sub/path` to
                // the workspace root for `@scope/pkg` and resolve the subpath
                // as a relative import from there.
                if let Some(resolved) = self.resolve_via_workspace(specifier) {
                    return Ok(resolved);
                }
                Err(ResolveError::NotFound {
                    specifier: specifier.to_string(),
                    from: from.to_path_buf(),
                    reason: Some(classify_unresolved_reason(specifier, &err.to_string())),
                })
            }
        }
    }

    /// Try to resolve a bare specifier via workspace package mappings.
    fn resolve_via_workspace(&self, specifier: &str) -> Option<ResolvedModule> {
        let pkg_name = dependency_name(specifier)?;
        let workspace_root = self.workspace_roots.get(&pkg_name)?;

        // Extract the subpath after the package name.
        let subpath = specifier.strip_prefix(&pkg_name)?;
        let subpath = subpath.strip_prefix('/').unwrap_or(subpath);

        if subpath.is_empty() {
            // Bare package import — resolve via the package's main entry.
            return resolve_with_extensions(workspace_root, "index");
        }

        // Deep subpath import — resolve as a file relative to the workspace root.
        resolve_with_extensions(workspace_root, subpath)
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
            err.reason().map_or_else(|| err.to_string(), |reason| reason.as_str().to_string())
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
    NotFound { specifier: String, from: PathBuf, reason: Option<UnresolvedReason> },
}

impl ResolveError {
    pub const fn reason(&self) -> Option<UnresolvedReason> {
        match self {
            Self::NotFound { reason, .. } => *reason,
        }
    }
}

fn classify_unresolved_reason(specifier: &str, error: &str) -> UnresolvedReason {
    // Runtime-specific built-in module prefixes.
    if specifier.starts_with("node:")
        || specifier.starts_with("bun:")
        || specifier.starts_with("deno:")
    {
        return UnresolvedReason::Externalized;
    }

    // Virtual/synthetic specifiers commonly used by bundler plugins.
    if specifier.starts_with("virtual:")
        || specifier.starts_with('\0')
        || specifier.starts_with('~')
    {
        return UnresolvedReason::UnsupportedSpecifier;
    }

    // Template literal syntax — cannot be statically resolved.
    if specifier.contains("${") {
        return UnresolvedReason::UnsupportedSpecifier;
    }

    // Glob / wildcard patterns.
    if specifier.contains('*') || specifier.contains('?') {
        return UnresolvedReason::UnsupportedSpecifier;
    }

    // Non-JS asset imports (CSS, images, etc.) that bundlers handle.
    if is_asset_specifier(specifier) {
        return UnresolvedReason::UnsupportedSpecifier;
    }

    // Node.js subpath imports (package.json "imports" field, e.g. `#utils/foo`).
    if specifier.starts_with('#') {
        if error.contains("exports") || error.contains("condition") || error.contains("imports") {
            return UnresolvedReason::ExportsConditionMiss;
        }
        // Subpath imports that failed resolution default to exports/condition miss
        // since they rely on the package.json "imports" map (same mechanism as exports).
        return UnresolvedReason::ExportsConditionMiss;
    }

    // Bare specifiers (not relative, not absolute).
    if !specifier.starts_with('.') && !specifier.starts_with('/') {
        if error.contains("tsconfig") || error.contains("paths") {
            return UnresolvedReason::TsconfigPathMiss;
        }
        if error.contains("exports") || error.contains("condition") {
            return UnresolvedReason::ExportsConditionMiss;
        }
        // A bare specifier with a subpath (e.g. `pkg/subpath`) where the package
        // has an exports field is most likely an exports-condition miss rather than
        // a missing file.
        if has_subpath(specifier) && error.contains("Package path") {
            return UnresolvedReason::ExportsConditionMiss;
        }
        // Looks like a valid package name — treat as a missing/uninstalled dependency
        // rather than a missing file.
        if looks_like_package_name(specifier) {
            return UnresolvedReason::MissingFile;
        }
        return UnresolvedReason::MissingFile;
    }

    UnresolvedReason::MissingFile
}

/// Check if a bare specifier includes a subpath beyond the package name.
fn has_subpath(specifier: &str) -> bool {
    let Some(pkg_name) = dependency_name(specifier) else {
        return false;
    };
    specifier.len() > pkg_name.len() && specifier[pkg_name.len()..].starts_with('/')
}

/// Check if a specifier looks like a non-JS asset import handled by bundlers.
fn is_asset_specifier(specifier: &str) -> bool {
    let ext = specifier.rsplit('.').next().unwrap_or("");
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "css" | "scss" | "sass" | "less" | "styl" | "stylus"
            | "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "avif"
            | "woff" | "woff2" | "ttf" | "eot" | "otf"
            | "mp4" | "webm" | "ogg" | "mp3" | "wav" | "flac"
            | "graphql" | "gql"
            | "yaml" | "yml" | "toml"
            | "txt" | "csv" | "xml"
            | "wasm"
    )
}

/// Heuristic: does this bare specifier look like a valid npm package name?
/// Scoped (`@scope/name`) or unscoped (`name`) with valid characters.
fn looks_like_package_name(specifier: &str) -> bool {
    let name_part = if let Some(rest) = specifier.strip_prefix('@') {
        // Scoped: expect `@scope/name` optionally followed by a subpath.
        match rest.find('/') {
            Some(idx) => {
                let after_scope = &rest[idx + 1..];
                // Get the package name portion (before any further `/`).
                after_scope.split('/').next().unwrap_or("")
            }
            None => return false, // `@scope` alone is not valid.
        }
    } else {
        specifier.split('/').next().unwrap_or("")
    };

    !name_part.is_empty()
        && !name_part.starts_with('.')
        && name_part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Resolve a subpath relative to a directory by trying common JS/TS extensions.
/// This bypasses `oxc_resolver` entirely to avoid tsconfig resolution issues.
fn resolve_with_extensions(root: &Path, subpath: &str) -> Option<ResolvedModule> {
    let base = root.join(subpath);

    // If the path already has an extension and exists, use it directly.
    if base.is_file() {
        return Some(ResolvedModule {
            path: base,
            via_exports: false,
            exports_subpath: None,
            exports_condition: None,
        });
    }

    // Try appending common extensions.
    for ext in &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"] {
        let candidate = base.with_extension(ext);
        if candidate.is_file() {
            return Some(ResolvedModule {
                path: candidate,
                via_exports: false,
                exports_subpath: None,
                exports_condition: None,
            });
        }
    }

    // Try as a directory with index file.
    if base.is_dir() {
        for ext in &["ts", "tsx", "js", "jsx"] {
            let candidate = base.join("index").with_extension(ext);
            if candidate.is_file() {
                return Some(ResolvedModule {
                    path: candidate,
                    via_exports: false,
                    exports_subpath: None,
                    exports_condition: None,
                });
            }
        }
    }

    None
}

fn resolved_via_package_exports(specifier: &str, resolution: &oxc_resolver::Resolution) -> bool {
    let Some(package_name) = dependency_name(specifier) else {
        return false;
    };
    let Some(package_json) = resolution.package_json() else {
        return false;
    };
    if package_json.exports().is_none() {
        return false;
    }

    match package_json.name() {
        Some(name) if name == package_name => true,
        _ => resolution.path().starts_with(package_json.directory()),
    }
}

/// Extract the matched exports subpath and condition from a resolution.
///
/// For a specifier like `@scope/pkg/utils` resolving via `"./utils": { "import": "./src/utils.js" }`,
/// this returns `(Some("./utils"), Some("import"))`.
fn extract_exports_attribution(
    specifier: &str,
    _resolution: &oxc_resolver::Resolution,
) -> (Option<String>, Option<String>) {
    let Some(package_name) = dependency_name(specifier) else {
        return (None, None);
    };

    // Derive the subpath the user asked for: `@scope/pkg/utils` -> `./utils`.
    let subpath = specifier
        .strip_prefix(&package_name)
        .map_or_else(|| ".".to_string(), |rest| {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            if rest.is_empty() { ".".to_string() } else { format!("./{rest}") }
        });

    // We report the subpath that was requested; the condition is not available
    // from the oxc_resolver resolution directly, so we leave it as None.
    // The subpath is sufficient for exports-aware attribution in the graph.
    (Some(subpath), None)
}
