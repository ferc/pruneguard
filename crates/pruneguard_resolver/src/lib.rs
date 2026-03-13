use std::path::{Path, PathBuf};

use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use pruneguard_config::ResolverConfig;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

/// Bump this whenever hardcoded resolver behaviour changes (e.g. `extension_alias`,
/// default extensions, condition names) so that the analysis cache is invalidated.
pub const RESOLVER_LOGIC_VERSION: u32 = 7;

/// Origin of a resolved alias, used for attribution in findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AliasOrigin {
    Manifest,
    #[serde(alias = "tsconfig")]
    TsconfigPaths,
    Webpack,
    Babel,
    Vite,
    #[default]
    FrameworkGenerated,
    SubpathImports,
    BrowserField,
}

impl AliasOrigin {
    /// Resolution priority — lower value means higher precedence.
    ///
    /// When multiple config-derived aliases match the same specifier, the
    /// alias with the highest precedence (lowest priority number) wins.
    /// Order: manifest exports > tsconfig paths > vite > webpack > babel > framework-generated.
    const fn priority(self) -> u8 {
        match self {
            Self::Manifest => 0,
            Self::SubpathImports => 1,
            Self::BrowserField => 2,
            Self::TsconfigPaths => 3,
            Self::Vite => 4,
            Self::Webpack => 5,
            Self::Babel => 6,
            Self::FrameworkGenerated => 7,
        }
    }
}

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
    /// Origin of the alias that resolved this module, if any.
    pub alias_origin: Option<AliasOrigin>,
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
    /// `require.resolve('specifier')` — resolves but doesn't import at runtime.
    RequireResolve,
    /// `import.meta.glob('pattern')` — Vite glob import (expanded to edges).
    ImportMetaGlob,
    /// JSDoc `@type {import('specifier')}` — type-level dependency.
    JsDocImport,
    /// `/// <reference path="..." />` — TypeScript file reference.
    TripleSlashFile,
    /// `/// <reference types="..." />` — TypeScript types reference.
    TripleSlashTypes,
    /// `import.meta.resolve('specifier')` — resolves a URL without importing.
    ImportMetaResolve,
    /// `require.context('./dir', ...)` — webpack dynamic context directory.
    RequireContext,
    /// `new URL('./worker.js', import.meta.url)` — worker/asset URL pattern.
    UrlConstructor,
    /// `import foo = require('bar')` — TypeScript import-equals.
    ImportEquals,
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
    /// Origin of the alias that resolved this edge, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_origin: Option<AliasOrigin>,
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
    /// The specifier targets a workspace package whose `exports` map does not
    /// expose the requested subpath.
    WorkspaceExportsMiss,
}

impl UnresolvedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingFile => "missing-file",
            Self::UnsupportedSpecifier => "unsupported-specifier",
            Self::TsconfigPathMiss => "tsconfig-path-miss",
            Self::ExportsConditionMiss => "exports-condition-miss",
            Self::Externalized => "externalized",
            Self::WorkspaceExportsMiss => "workspace-exports-miss",
        }
    }

    /// Whether this reason represents a "benign" unresolved specifier that
    /// should not count toward confidence-lowering pressure thresholds.
    pub const fn is_benign(self) -> bool {
        matches!(self, Self::UnsupportedSpecifier | Self::Externalized)
    }
}

/// Fallback main fields from a workspace package.json.
#[derive(Debug, Clone, Default)]
struct WorkspaceMainFields {
    /// `"typings"` or `"types"` field — path to declaration entry.
    types_entry: Option<String>,
    /// `"module"` field — ESM entry.
    module_entry: Option<String>,
}

/// Module resolver built on `oxc_resolver`.
pub struct ModuleResolver {
    inner: Resolver,
    /// Map of workspace package names to their root directories.
    /// Enables resolution of deep subpath imports like `@scope/pkg/path/to/file`.
    workspace_roots: FxHashMap<String, PathBuf>,
    /// Cached workspace package.json exports maps, used to validate subpath
    /// imports against the package's declared public API.
    workspace_exports: FxHashMap<String, Option<serde_json::Value>>,
    /// Cached workspace package.json `imports` maps (subpath imports),
    /// keyed by workspace root directory.  Used to resolve `#`-prefixed specifiers.
    workspace_imports: FxHashMap<PathBuf, Option<serde_json::Value>>,
    /// Cached workspace package.json `browser` field mappings.
    /// `browser` can be a string (alternative main) or an object (per-module replacements).
    workspace_browser: FxHashMap<String, Option<serde_json::Value>>,
    /// Cached workspace package.json fallback main fields (`typings`, `types`, `module`).
    workspace_main_fields: FxHashMap<String, WorkspaceMainFields>,
    /// Aliases from framework config adapters (e.g. Vite resolve.alias, Webpack resolve.alias).
    /// Each entry maps a prefix pattern to an absolute target directory, along with its origin.
    config_aliases: Vec<(String, PathBuf, AliasOrigin)>,
    /// External packages from framework config adapters.
    /// Specifiers matching these names are treated as external dependencies.
    config_externals: FxHashSet<String>,
    /// Specifier patterns that should be treated as resolved (e.g. virtual modules).
    /// Matching specifiers will not be counted as unresolved.
    ignore_unresolved: Vec<String>,
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
        Self {
            inner,
            workspace_roots: FxHashMap::default(),
            workspace_exports: FxHashMap::default(),
            workspace_imports: FxHashMap::default(),
            workspace_browser: FxHashMap::default(),
            workspace_main_fields: FxHashMap::default(),
            config_aliases: Vec::new(),
            config_externals: FxHashSet::default(),
            ignore_unresolved: Vec::new(),
        }
    }

    /// Register workspace package name -> root directory mappings.
    /// This enables resolution of deep subpath imports into workspace packages
    /// (e.g. `@calcom/features/auth/lib/getLocale` -> `packages/features/auth/lib/getLocale.ts`).
    pub fn set_workspace_roots(&mut self, roots: FxHashMap<String, PathBuf>) {
        for (pkg_name, root) in &roots {
            let exports = load_workspace_exports(root);
            self.workspace_exports.insert(pkg_name.clone(), exports);
            let imports = load_workspace_imports(root);
            self.workspace_imports.insert(root.clone(), imports);
            let browser = load_workspace_field(root, "browser");
            self.workspace_browser.insert(pkg_name.clone(), browser);
            let main_fields = load_workspace_main_fields(root);
            self.workspace_main_fields.insert(pkg_name.clone(), main_fields);
        }
        self.workspace_roots = roots;
    }

    /// Register aliases from framework config adapters (e.g. Vite resolve.alias).
    ///
    /// Each alias maps a prefix (e.g. `@`) to a target directory (e.g. `./src`),
    /// along with an `AliasOrigin` indicating which config source it came from.
    /// Relative targets are resolved against `project_root`.
    pub fn set_config_aliases(
        &mut self,
        aliases: Vec<(String, String, AliasOrigin)>,
        project_root: &Path,
    ) {
        self.config_aliases = aliases
            .into_iter()
            .map(|(pattern, target, origin)| {
                let abs = if Path::new(&target).is_absolute() {
                    PathBuf::from(&target)
                } else {
                    let stripped = target.strip_prefix("./").unwrap_or(&target);
                    project_root.join(stripped)
                };
                (pattern, abs, origin)
            })
            .collect();

        // Sort by origin priority so higher-precedence aliases are tried first.
        // Uses a stable sort to preserve insertion order among same-origin aliases.
        self.config_aliases.sort_by_key(|(_, _, origin)| origin.priority());
    }

    /// Register external packages from framework config adapters.
    ///
    /// Specifiers matching these names are treated as external dependencies
    /// without attempting filesystem resolution.
    pub fn set_config_externals(&mut self, externals: Vec<String>) {
        self.config_externals = externals.into_iter().collect();
    }

    /// Register specifier patterns that should be treated as resolved.
    ///
    /// Matching specifiers (by prefix) will not be counted as unresolved,
    /// which is useful for virtual modules, framework-specific aliases, etc.
    pub fn set_ignore_unresolved(&mut self, patterns: Vec<String>) {
        self.ignore_unresolved = patterns;
    }

    /// Check if a specifier matches any ignore_unresolved pattern.
    pub fn is_ignored_unresolved(&self, specifier: &str) -> bool {
        self.ignore_unresolved
            .iter()
            .any(|pattern| specifier == pattern || specifier.starts_with(pattern))
    }

    /// Resolve a module specifier from a given file.
    pub fn resolve(&self, specifier: &str, from: &Path) -> Result<ResolvedModule, ResolveError> {
        // Check ignore_unresolved patterns first — treat matching specifiers as external.
        if self.is_ignored_unresolved(specifier) {
            return Err(ResolveError::NotFound {
                specifier: specifier.to_string(),
                from: from.to_path_buf(),
                reason: Some(UnresolvedReason::Externalized),
            });
        }

        // Check config-derived externals.
        if let Some(pkg) = dependency_name(specifier)
            && self.config_externals.contains(&pkg)
        {
            return Err(ResolveError::NotFound {
                specifier: specifier.to_string(),
                from: from.to_path_buf(),
                reason: Some(UnresolvedReason::Externalized),
            });
        }

        // Try config-derived aliases (e.g. Vite resolve.alias).
        if let Some(resolved) = self.resolve_via_config_alias(specifier, from) {
            return Ok(resolved);
        }

        // Resolve `#`-prefixed subpath imports via the workspace's package.json
        // `imports` field before falling through to the standard resolver.
        if specifier.starts_with('#')
            && let Some(resolved) = self.resolve_subpath_import(specifier, from)
        {
            return Ok(resolved);
        }

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
                    alias_origin: None,
                })
            }
            Err(err) => {
                // Try browser field remapping for workspace packages.
                if let Some(resolved) = self.resolve_via_browser_field(specifier, from) {
                    return Ok(resolved);
                }
                // Try workspace-aware resolution: map `@scope/pkg/sub/path` to
                // the workspace root for `@scope/pkg` and resolve the subpath
                // as a relative import from there.
                if let Some(resolved) = self.resolve_via_workspace(specifier) {
                    return Ok(resolved);
                }
                // Classify the reason; use WorkspaceExportsMiss when appropriate.
                let reason = if self.is_workspace_exports_miss(specifier) {
                    UnresolvedReason::WorkspaceExportsMiss
                } else {
                    classify_unresolved_reason(specifier, &err.to_string())
                };
                Err(ResolveError::NotFound {
                    specifier: specifier.to_string(),
                    from: from.to_path_buf(),
                    reason: Some(reason),
                })
            }
        }
    }

    /// Try to resolve a specifier via config-derived aliases.
    ///
    /// For each alias `(pattern, target_dir, origin)`, if the specifier starts with
    /// `pattern`, replace the prefix and resolve the remainder relative to the
    /// target directory.  The `AliasOrigin` is attached to the returned module.
    fn resolve_via_config_alias(&self, specifier: &str, _from: &Path) -> Option<ResolvedModule> {
        for (pattern, target_dir, origin) in &self.config_aliases {
            // Exact match: `@` → `./src` means `@` resolves to `./src/index`
            if specifier == pattern {
                return resolve_with_extensions(target_dir, "index").map(|mut m| {
                    m.alias_origin = Some(*origin);
                    m
                });
            }
            // Prefix match: `@/utils` with alias `@` → `./src` means `./src/utils`
            let prefix_with_sep = format!("{pattern}/");
            if let Some(rest) = specifier.strip_prefix(&prefix_with_sep) {
                return resolve_with_extensions(target_dir, rest).map(|mut m| {
                    m.alias_origin = Some(*origin);
                    m
                });
            }
        }
        None
    }

    /// Try to resolve a bare specifier via workspace package mappings.
    ///
    /// When the workspace package has an `exports` map, the specifier is validated
    /// against it to avoid creating false edges to private internal files.  If the
    /// exports map provides a concrete file target we use that; otherwise we fall
    /// back to filesystem probing only when the subpath is declared (or there is no
    /// exports map at all).
    fn resolve_via_workspace(&self, specifier: &str) -> Option<ResolvedModule> {
        let pkg_name = dependency_name(specifier)?;
        let workspace_root = self.workspace_roots.get(&pkg_name)?;

        // Extract the subpath after the package name.
        let subpath = specifier.strip_prefix(&pkg_name)?;
        let subpath = subpath.strip_prefix('/').unwrap_or(subpath);
        let exports_key = if subpath.is_empty() { ".".to_string() } else { format!("./{subpath}") };

        // If the workspace package has an exports map, validate the subpath.
        if let Some(Some(exports_value)) = self.workspace_exports.get(&pkg_name) {
            if !exports_map_defines_subpath(exports_value, &exports_key) {
                return None;
            }
            // Try to resolve via the exports map target before filesystem probing.
            if let Some(resolved) =
                resolve_from_exports_value(exports_value, &exports_key, workspace_root)
            {
                return Some(resolved);
            }
        }

        if subpath.is_empty() {
            // Bare package import -- try `module` and `typings`/`types` fields,
            // then fall back to `index`.
            if let Some(main_fields) = self.workspace_main_fields.get(&pkg_name) {
                if let Some(module_entry) = &main_fields.module_entry {
                    let relative = module_entry.strip_prefix("./").unwrap_or(module_entry);
                    if let Some(resolved) = resolve_with_extensions(workspace_root, relative) {
                        return Some(resolved);
                    }
                }
                if let Some(types_entry) = &main_fields.types_entry {
                    let relative = types_entry.strip_prefix("./").unwrap_or(types_entry);
                    if let Some(resolved) = resolve_with_extensions(workspace_root, relative) {
                        return Some(resolved);
                    }
                }
            }
            return resolve_with_extensions(workspace_root, "index");
        }

        // Deep subpath import -- resolve as a file relative to the workspace root.
        resolve_with_extensions(workspace_root, subpath)
    }

    /// Try to resolve a `#`-prefixed subpath import via the workspace's
    /// package.json `imports` field.
    ///
    /// Finds the workspace root that contains `from`, looks up the `imports`
    /// map, and resolves the alias to a concrete file path.
    fn resolve_subpath_import(&self, specifier: &str, from: &Path) -> Option<ResolvedModule> {
        // Find the workspace whose root contains the importing file.
        let (workspace_root, imports_value) = self
            .workspace_imports
            .iter()
            .find(|(root, _)| from.starts_with(root))
            .and_then(|(root, imports)| Some((root, imports.as_ref()?)))?;

        let resolved_path = resolve_imports_alias(imports_value, specifier)?;

        // The resolved path is relative (e.g. `./src/utils/index.ts`).
        let relative = resolved_path.strip_prefix("./").unwrap_or(&resolved_path);
        let candidate = workspace_root.join(relative);

        if candidate.is_file() {
            return Some(ResolvedModule {
                path: candidate,
                via_exports: false,
                exports_subpath: None,
                exports_condition: None,
                alias_origin: Some(AliasOrigin::SubpathImports),
            });
        }

        resolve_with_extensions(workspace_root, relative).map(|mut m| {
            m.alias_origin = Some(AliasOrigin::SubpathImports);
            m
        })
    }

    /// Try to resolve a specifier via a workspace package's `browser` field.
    ///
    /// When the `browser` field is an object, it maps module specifiers to
    /// replacement paths (or `false` to stub them out).  When it is a string,
    /// it specifies an alternative main entry point.
    fn resolve_via_browser_field(&self, specifier: &str, from: &Path) -> Option<ResolvedModule> {
        // Find which workspace the importing file belongs to.
        let (pkg_name, workspace_root) =
            self.workspace_roots.iter().find(|(_, root)| from.starts_with(root))?;
        let browser = self.workspace_browser.get(pkg_name)?.as_ref()?;

        match browser {
            serde_json::Value::String(alt_main) if specifier == pkg_name.as_str() => {
                let relative = alt_main.strip_prefix("./").unwrap_or(alt_main.as_str());
                resolve_with_extensions(workspace_root, relative).map(|mut m| {
                    m.alias_origin = Some(AliasOrigin::BrowserField);
                    m
                })
            }
            serde_json::Value::Object(map) => {
                // Look for an exact match of the specifier in the browser map.
                let replacement = map.get(specifier).and_then(|v| v.as_str())?;
                let relative = replacement.strip_prefix("./").unwrap_or(replacement);
                let candidate = workspace_root.join(relative);
                if candidate.is_file() {
                    Some(ResolvedModule {
                        path: candidate,
                        via_exports: false,
                        exports_subpath: None,
                        exports_condition: None,
                        alias_origin: Some(AliasOrigin::BrowserField),
                    })
                } else {
                    resolve_with_extensions(workspace_root, relative).map(|mut m| {
                        m.alias_origin = Some(AliasOrigin::BrowserField);
                        m
                    })
                }
            }
            _ => None,
        }
    }

    /// Check whether a specifier targets a workspace package whose `exports` map
    /// does not expose the requested subpath.  Used to emit an
    /// `UnresolvedReason::WorkspaceExportsMiss` diagnostic.
    fn is_workspace_exports_miss(&self, specifier: &str) -> bool {
        let Some(pkg_name) = dependency_name(specifier) else {
            return false;
        };
        let Some(Some(exports_value)) = self.workspace_exports.get(&pkg_name) else {
            return false;
        };
        let subpath = specifier.strip_prefix(&pkg_name).unwrap_or("");
        let subpath = subpath.strip_prefix('/').unwrap_or(subpath);
        let exports_key = if subpath.is_empty() { ".".to_string() } else { format!("./{subpath}") };
        !exports_map_defines_subpath(exports_value, &exports_key)
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

    // Template literal syntax -- cannot be statically resolved.
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
        // Looks like a valid package name -- treat as a missing/uninstalled dependency
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
        "css"
            | "scss"
            | "sass"
            | "less"
            | "styl"
            | "stylus"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "ico"
            | "webp"
            | "avif"
            | "woff"
            | "woff2"
            | "ttf"
            | "eot"
            | "otf"
            | "mp4"
            | "webm"
            | "ogg"
            | "mp3"
            | "wav"
            | "flac"
            | "graphql"
            | "gql"
            | "yaml"
            | "yml"
            | "toml"
            | "txt"
            | "csv"
            | "xml"
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
            alias_origin: None,
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
                alias_origin: None,
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
                    alias_origin: None,
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
    let subpath = specifier.strip_prefix(&package_name).map_or_else(
        || ".".to_string(),
        |rest| {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            if rest.is_empty() { ".".to_string() } else { format!("./{rest}") }
        },
    );

    // We report the subpath that was requested; the condition is not available
    // from the oxc_resolver resolution directly, so we leave it as None.
    // The subpath is sufficient for exports-aware attribution in the graph.
    (Some(subpath), None)
}

// ---------------------------------------------------------------------------
// Workspace exports helpers
// ---------------------------------------------------------------------------

/// Load the `exports` field from a workspace package's `package.json`.
fn load_workspace_exports(root: &Path) -> Option<serde_json::Value> {
    let pkg_json_path = root.join("package.json");
    let content = std::fs::read_to_string(pkg_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("exports").cloned()
}

/// Load the `imports` field from a workspace package's `package.json`.
fn load_workspace_imports(root: &Path) -> Option<serde_json::Value> {
    let pkg_json_path = root.join("package.json");
    let content = std::fs::read_to_string(pkg_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("imports").cloned()
}

/// Load an arbitrary field from a workspace package's `package.json`.
fn load_workspace_field(root: &Path, field: &str) -> Option<serde_json::Value> {
    let pkg_json_path = root.join("package.json");
    let content = std::fs::read_to_string(pkg_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get(field).cloned()
}

/// Load `typings`/`types` and `module` fields from a workspace's package.json.
fn load_workspace_main_fields(root: &Path) -> WorkspaceMainFields {
    let pkg_json_path = root.join("package.json");
    let content = match std::fs::read_to_string(pkg_json_path) {
        Ok(c) => c,
        Err(_) => return WorkspaceMainFields::default(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return WorkspaceMainFields::default(),
    };
    let types_entry = parsed
        .get("typings")
        .or_else(|| parsed.get("types"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let module_entry = parsed.get("module").and_then(|v| v.as_str()).map(ToString::to_string);
    WorkspaceMainFields { types_entry, module_entry }
}

/// Resolve a `#`-prefixed specifier against an `imports` map value.
///
/// Supports exact matches, condition maps (`import`/`require`/`default`),
/// and wildcard patterns (e.g. `#utils/*` matching `#utils/foo`).
fn resolve_imports_alias(imports: &serde_json::Value, specifier: &str) -> Option<String> {
    let map = imports.as_object()?;

    // Exact match.
    if let Some(value) = map.get(specifier) {
        return resolve_imports_value(value);
    }

    // Wildcard pattern match.
    for (pattern, value) in map {
        if let Some(prefix) = pattern.strip_suffix('*')
            && let Some(rest) = specifier.strip_prefix(prefix)
        {
            return resolve_imports_value(value).map(|target| target.replace('*', rest));
        }
    }

    None
}

/// Resolve a single value from an `imports` field entry.
/// The value can be a string, a condition map, or an array (first match wins).
fn resolve_imports_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            // Condition map — try common conditions in priority order.
            for condition in &["import", "require", "default"] {
                if let Some(v) = map.get(*condition) {
                    return resolve_imports_value(v);
                }
            }
            map.values().find_map(resolve_imports_value)
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(resolve_imports_value),
        _ => None,
    }
}

/// Check whether an exports map defines a given subpath (exact or wildcard).
fn exports_map_defines_subpath(exports: &serde_json::Value, subpath: &str) -> bool {
    match exports {
        serde_json::Value::Object(map) => {
            let is_subpath_map = map.keys().any(|k| k.starts_with('.'));
            if !is_subpath_map {
                // Condition-only map applies only to the "." subpath.
                return subpath == ".";
            }
            if map.contains_key(subpath) {
                return true;
            }
            // Wildcard/pattern match (e.g. `./*` matches `./foo/bar`).
            map.keys().any(|pattern| {
                pattern.strip_suffix('*').is_some_and(|prefix| subpath.starts_with(prefix))
            })
        }
        serde_json::Value::String(_) => subpath == ".",
        _ => false,
    }
}

/// Resolve a subpath from the exports map to a concrete file.
/// Walks the condition tree preferring `types` then `import` then `require` then `default`.
fn resolve_from_exports_value(
    exports: &serde_json::Value,
    subpath: &str,
    workspace_root: &Path,
) -> Option<ResolvedModule> {
    let target = lookup_exports_target(exports, subpath)?;
    let relative = target.strip_prefix("./").unwrap_or(&target);
    let candidate = workspace_root.join(relative);
    if candidate.is_file() {
        return Some(ResolvedModule {
            path: candidate,
            via_exports: true,
            exports_subpath: Some(subpath.to_string()),
            exports_condition: None,
            alias_origin: Some(AliasOrigin::Manifest),
        });
    }
    resolve_with_extensions(workspace_root, relative).map(|mut m| {
        m.via_exports = true;
        m.exports_subpath = Some(subpath.to_string());
        m.alias_origin = Some(AliasOrigin::Manifest);
        m
    })
}

/// Walk the exports value to find the string target for a given subpath.
fn lookup_exports_target(exports: &serde_json::Value, subpath: &str) -> Option<String> {
    match exports {
        serde_json::Value::String(s) if subpath == "." => Some(s.clone()),
        serde_json::Value::Object(map) => {
            let is_subpath_map = map.keys().any(|k| k.starts_with('.'));
            if !is_subpath_map {
                if subpath == "." {
                    return resolve_condition_map(map);
                }
                return None;
            }
            if let Some(value) = map.get(subpath) {
                return resolve_exports_value(value);
            }
            for (pattern, value) in map {
                if let Some(prefix) = pattern.strip_suffix('*')
                    && let Some(rest) = subpath.strip_prefix(prefix)
                    && let Some(target) = resolve_exports_value(value)
                {
                    return Some(target.replace('*', rest));
                }
            }
            None
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| lookup_exports_target(v, subpath)),
        _ => None,
    }
}

/// Resolve a single exports value (string, condition map, or array).
fn resolve_exports_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => resolve_condition_map(map),
        serde_json::Value::Array(arr) => arr.iter().find_map(resolve_exports_value),
        _ => None,
    }
}

/// Select the best target from a condition map.
/// Priority: `types` then `import` then `require` then `node` then `default`.
/// We prefer `types` first because pruneguard is a static analysis tool and
/// `.d.ts` targets give accurate type-level export surfaces.
fn resolve_condition_map(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    for condition in &["types", "import", "require", "node", "default"] {
        if let Some(value) = map.get(*condition) {
            return resolve_exports_value(value);
        }
    }
    map.values().find_map(resolve_exports_value)
}
