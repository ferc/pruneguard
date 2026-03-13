use std::path::{Path, PathBuf};

use pruneguard_manifest::PackageManifest;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// How a framework classifies a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileClassification {
    Test,
    Story,
    Config,
    Generated,
}

/// Confidence level for a framework detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DetectionConfidence {
    /// Config file + dependency confirmed.
    Exact,
    /// Directory conventions only or partial signals.
    Heuristic,
}

/// Rich detection result from a framework pack.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkDetection {
    pub name: &'static str,
    pub confidence: DetectionConfidence,
    pub signals: Vec<String>,
    pub reasons: Vec<String>,
}

/// A seed entrypoint contributed by a framework pack.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkEntrypointSeed {
    pub path: PathBuf,
    pub profile: Option<&'static str>,
    pub kind: &'static str,
    pub reason: String,
    pub heuristic: bool,
}

/// A glob pattern that maps files to a classification.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkClassificationRule {
    pub pattern: String,
    pub classification: FileClassification,
}

/// A trust note warning the user about heuristic or incomplete detection.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkTrustNote {
    pub message: String,
    pub affects: TrustNoteScope,
}

/// Scope of a trust note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TrustNoteScope {
    AllFindings,
    EntrypointsOnly,
    Workspace(String),
    Path(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A framework detection pack that contributes entrypoints and ignore patterns.
pub trait FrameworkPack: Send + Sync {
    /// Name of this framework.
    fn name(&self) -> &'static str;

    /// Check if this framework is active for a given workspace.
    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool;

    /// Return entrypoint files contributed by this framework.
    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf>;

    /// Return additional ignore patterns contributed by this framework.
    fn ignore_patterns(&self) -> Vec<String>;

    /// Return file classification overrides (e.g., treat `.stories.tsx` as story files).
    fn file_kinds(&self) -> Vec<(String, FileClassification)>;

    // -- New methods with default impls --

    /// Rich detection with confidence levels and signals.
    fn detect_detailed(
        &self,
        _workspace_root: &Path,
        _manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        None
    }

    /// Typed entrypoint seeds with metadata.
    fn entrypoint_seeds(&self, _workspace_root: &Path) -> Vec<FrameworkEntrypointSeed> {
        vec![]
    }

    /// Glob-based classification rules.
    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![]
    }

    /// Trust notes describing heuristic limitations.
    fn trust_notes(
        &self,
        _workspace_root: &Path,
        _manifest: &PackageManifest,
    ) -> Vec<FrameworkTrustNote> {
        vec![]
    }

    /// Patterns matching generated/build output that should be treated as generated.
    fn generated_output_patterns(&self) -> Vec<String> {
        vec![]
    }

    /// Patterns for files that the framework auto-loads (auto-imports).
    fn auto_loaded_patterns(&self) -> Vec<String> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get all built-in framework packs.
pub fn built_in_packs() -> Vec<Box<dyn FrameworkPack>> {
    vec![
        // Original packs
        Box::new(NextPack),
        Box::new(VitePack),
        Box::new(VitestPack),
        Box::new(JestPack),
        Box::new(StorybookPack),
        Box::new(FileBasedRoutingPack),
        Box::new(RootConfigPack),
        // Tier 1 — App Frameworks
        Box::new(NuxtPack),
        Box::new(AstroPack),
        Box::new(SvelteKitPack),
        Box::new(RemixPack),
        // Tier 2 — Monorepo / Build Systems
        Box::new(NxPack),
        Box::new(TurboPack),
        Box::new(AngularPack),
        // Tier 3 — Dev / Runtime Tooling
        Box::new(PlaywrightPack),
        Box::new(CypressPack),
        Box::new(VitePressPack),
        Box::new(DocusaurusPack),
    ]
}

/// Detect all active frameworks in a workspace, returning rich detection info.
pub fn detect_all_frameworks(
    workspace_root: &Path,
    manifest: &PackageManifest,
) -> Vec<FrameworkDetection> {
    built_in_packs()
        .iter()
        .filter_map(|pack| {
            pack.detect_detailed(workspace_root, manifest).or_else(|| {
                // Fall back to the boolean detect() for packs that haven't
                // implemented detect_detailed() yet.
                if pack.detect(workspace_root, manifest) {
                    Some(FrameworkDetection {
                        name: pack.name(),
                        confidence: DetectionConfidence::Heuristic,
                        signals: vec![],
                        reasons: vec!["detected via legacy detect() method".into()],
                    })
                } else {
                    None
                }
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn manifest_has_any_dep(manifest: &PackageManifest, deps: &[&str]) -> bool {
    let check =
        |d: &rustc_hash::FxHashMap<String, String>| deps.iter().any(|dep| d.contains_key(*dep));
    manifest.dependencies.as_ref().is_some_and(check)
        || manifest.dev_dependencies.as_ref().is_some_and(check)
}

fn manifest_has_dep(manifest: &PackageManifest, dep: &str) -> bool {
    manifest_has_any_dep(manifest, &[dep])
}

fn manifest_has_dev_dep(manifest: &PackageManifest, dep: &str) -> bool {
    manifest.dev_dependencies.as_ref().is_some_and(|d| d.contains_key(dep))
}

/// Recursively collect JS/TS files from a directory.
fn collect_files_recursive(dir: &Path, entries: &mut Vec<PathBuf>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, entries);
        } else if pruneguard_fs::has_js_ts_extension(&path) {
            entries.push(path);
        }
    }
}

/// Collect `*.stories.*` and `*.story.*` files recursively.
fn collect_story_files(dir: &Path, entries: &mut Vec<PathBuf>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip common non-source directories
            if matches!(
                name,
                "node_modules" | ".git" | "dist" | ".next" | ".storybook" | "storybook-static"
            ) {
                continue;
            }
            collect_story_files(&path, entries);
        } else if path.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
            (name.contains(".stories.") || name.contains(".story."))
                && pruneguard_fs::has_js_ts_extension(&path)
        }) {
            entries.push(path);
        }
    }
}

/// Recursively collect `*.spec.*` files from a directory.
fn collect_spec_files_recursive(dir: &Path, entries: &mut Vec<PathBuf>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_spec_files_recursive(&path, entries);
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.contains(".spec."))
        {
            entries.push(path);
        }
    }
}

/// Recursively collect `SvelteKit` route files (`+page.*`, `+layout.*`, etc.).
fn collect_sveltekit_route_files(dir: &Path, entries: &mut Vec<PathBuf>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sveltekit_route_files(&path, entries);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && (name.starts_with("+page.")
                || name.starts_with("+layout.")
                || name.starts_with("+server.")
                || name.starts_with("+error."))
        {
            entries.push(path);
        }
    }
}

/// Check if any config file matching a base name with common extensions exists.
fn find_config_file(workspace_root: &Path, base: &str) -> Option<PathBuf> {
    for ext in &["ts", "js", "mjs", "cjs", "mts"] {
        let path = workspace_root.join(format!("{base}.{ext}"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Collect all existing config file variants into entries.
fn collect_config_variants(workspace_root: &Path, base: &str, entries: &mut Vec<PathBuf>) {
    for ext in &["ts", "js", "mjs", "cjs", "mts"] {
        let path = workspace_root.join(format!("{base}.{ext}"));
        if path.exists() {
            entries.push(path);
        }
    }
}

/// Build detection signals from dependency and config file presence.
fn build_detection_signals(
    workspace_root: &Path,
    manifest: &PackageManifest,
    dep_names: &[&str],
    config_bases: &[&str],
) -> (bool, bool, Vec<String>) {
    let mut signals = Vec::new();
    let mut has_dep = false;
    let mut has_config = false;

    for dep in dep_names {
        if manifest_has_any_dep(manifest, &[dep]) {
            signals.push(format!("dependency `{dep}` found in package.json"));
            has_dep = true;
        }
    }

    for base in config_bases {
        if find_config_file(workspace_root, base).is_some() {
            signals.push(format!("config file `{base}.*` found"));
            has_config = true;
        }
    }

    (has_dep, has_config, signals)
}

const fn detection_confidence(has_dep: bool, has_config: bool) -> DetectionConfidence {
    if has_dep && has_config { DetectionConfidence::Exact } else { DetectionConfidence::Heuristic }
}

// ===========================================================================
// Built-in packs — Original
// ===========================================================================

struct NextPack;

impl FrameworkPack for NextPack {
    fn name(&self) -> &'static str {
        "next"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_any_dep(manifest, &["next"])
            || workspace_root.join("next.config.js").exists()
            || workspace_root.join("next.config.mjs").exists()
            || workspace_root.join("next.config.ts").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["next"], &["next.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Next.js app framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // App Router: ALL files under app/ are framework entrypoints
        // (page.tsx, layout.tsx, route.ts, loading.tsx, error.tsx, template.tsx, etc.)
        let app_dir = workspace_root.join("app");
        if app_dir.exists() {
            collect_files_recursive(&app_dir, &mut entries);
        }

        // Pages Router: ALL files under pages/ are framework entrypoints
        let pages_dir = workspace_root.join("pages");
        if pages_dir.exists() {
            collect_files_recursive(&pages_dir, &mut entries);
        }

        // next.config
        for name in &["next.config.js", "next.config.mjs", "next.config.ts"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        // Next.js instrumentation hooks (auto-loaded by the framework)
        for name in &[
            "instrumentation.ts",
            "instrumentation.js",
            "instrumentation-client.ts",
            "instrumentation-client.js",
            "src/instrumentation.ts",
            "src/instrumentation.js",
        ] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        // Next.js middleware
        for name in &["middleware.ts", "middleware.js", "src/middleware.ts", "src/middleware.js"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec![".next/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec![".next/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct VitePack;

impl FrameworkPack for VitePack {
    fn name(&self) -> &'static str {
        "vite"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "vite")
            || workspace_root.join("vite.config.ts").exists()
            || workspace_root.join("vite.config.js").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["vite"], &["vite.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Vite build tool detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();
        for name in &["vite.config.ts", "vite.config.js", "vite.config.mts"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }
        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["dist/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["dist/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct VitestPack;

impl FrameworkPack for VitestPack {
    fn name(&self) -> &'static str {
        "vitest"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "vitest") || workspace_root.join("vitest.config.ts").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["vitest"], &["vitest.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Vitest test framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();
        for name in &["vitest.config.ts", "vitest.config.js", "vitest.config.mts"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }
        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        vec![
            ("**/*.test.*".to_string(), FileClassification::Test),
            ("**/*.spec.*".to_string(), FileClassification::Test),
            ("**/__tests__/**".to_string(), FileClassification::Test),
        ]
    }

    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![
            FrameworkClassificationRule {
                pattern: "**/*.test.*".into(),
                classification: FileClassification::Test,
            },
            FrameworkClassificationRule {
                pattern: "**/*.spec.*".into(),
                classification: FileClassification::Test,
            },
            FrameworkClassificationRule {
                pattern: "**/__tests__/**".into(),
                classification: FileClassification::Test,
            },
        ]
    }
}

// ---------------------------------------------------------------------------

struct JestPack;

impl FrameworkPack for JestPack {
    fn name(&self) -> &'static str {
        "jest"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "jest")
            || workspace_root.join("jest.config.js").exists()
            || workspace_root.join("jest.config.ts").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["jest"], &["jest.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Jest test framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();
        for name in &["jest.config.js", "jest.config.ts", "jest.config.cjs", "jest.config.mjs"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }
        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        vec![
            ("**/*.test.*".to_string(), FileClassification::Test),
            ("**/*.spec.*".to_string(), FileClassification::Test),
            ("**/__tests__/**".to_string(), FileClassification::Test),
        ]
    }

    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![
            FrameworkClassificationRule {
                pattern: "**/*.test.*".into(),
                classification: FileClassification::Test,
            },
            FrameworkClassificationRule {
                pattern: "**/*.spec.*".into(),
                classification: FileClassification::Test,
            },
            FrameworkClassificationRule {
                pattern: "**/__tests__/**".into(),
                classification: FileClassification::Test,
            },
        ]
    }
}

// ---------------------------------------------------------------------------

struct StorybookPack;

impl FrameworkPack for StorybookPack {
    fn name(&self) -> &'static str {
        "storybook"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest.dev_dependencies.as_ref().is_some_and(|d| {
            d.contains_key("@storybook/react")
                || d.contains_key("@storybook/vue3")
                || d.contains_key("@storybook/angular")
                || d.contains_key("storybook")
        }) || workspace_root.join(".storybook").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let mut signals = Vec::new();
        let mut has_dep = false;

        for dep in &["storybook", "@storybook/react", "@storybook/vue3", "@storybook/angular"] {
            if manifest_has_dev_dep(manifest, dep) {
                signals.push(format!("devDependency `{dep}` found"));
                has_dep = true;
            }
        }

        let has_config = workspace_root.join(".storybook").exists();
        if has_config {
            signals.push("`.storybook/` directory found".into());
        }

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Storybook component explorer detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();
        let storybook_dir = workspace_root.join(".storybook");
        if storybook_dir.exists() {
            for name in &[
                "main.ts",
                "main.tsx",
                "main.js",
                "main.jsx",
                "preview.ts",
                "preview.tsx",
                "preview.js",
                "preview.jsx",
            ] {
                let path = storybook_dir.join(name);
                if path.exists() {
                    entries.push(path);
                }
            }
        }

        // Story files are auto-discovered by Storybook via its glob config.
        // Each story file is an independent entrypoint.
        collect_story_files(workspace_root, &mut entries);

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["storybook-static/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        vec![
            ("**/*.stories.*".to_string(), FileClassification::Story),
            ("**/*.story.*".to_string(), FileClassification::Story),
        ]
    }

    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![
            FrameworkClassificationRule {
                pattern: "**/*.stories.*".into(),
                classification: FileClassification::Story,
            },
            FrameworkClassificationRule {
                pattern: "**/*.story.*".into(),
                classification: FileClassification::Story,
            },
        ]
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["storybook-static/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

/// Generic file-based routing pack.
/// Detects any framework that uses file-system routing conventions
/// (`TanStack` Router, Remix, Expo Router, `SvelteKit`, Solid Start, etc.).
struct FileBasedRoutingPack;

/// Known file-based router dependencies.
const FILE_ROUTER_DEPS: &[&str] = &[
    "@tanstack/react-router",
    "@tanstack/react-start",
    "@tanstack/solid-router",
    "@remix-run/react",
    "@remix-run/node",
    "remix",
    "expo-router",
    "@solidjs/start",
];

impl FrameworkPack for FileBasedRoutingPack {
    fn name(&self) -> &'static str {
        "file-routing"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        let has_router = manifest_has_any_dep(manifest, FILE_ROUTER_DEPS);

        // Also detect by convention if routes/ directory exists alongside a router
        has_router
            || (workspace_root.join("src/routes").exists()
                || workspace_root.join("app/routes").exists())
                && manifest_has_any_dep(
                    manifest,
                    &["react-router", "react-router-dom", "vue-router"],
                )
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let mut signals = Vec::new();

        let has_router = manifest_has_any_dep(manifest, FILE_ROUTER_DEPS);
        if has_router {
            for dep in FILE_ROUTER_DEPS {
                if manifest_has_any_dep(manifest, &[dep]) {
                    signals.push(format!("file-based router dependency `{dep}` found"));
                }
            }
        }

        let has_routes_dir = workspace_root.join("src/routes").exists()
            || workspace_root.join("app/routes").exists();
        if has_routes_dir {
            signals.push("routes/ directory found".into());
        }

        let has_conventional = has_routes_dir
            && manifest_has_any_dep(manifest, &["react-router", "react-router-dom", "vue-router"]);

        if !has_router && !has_conventional {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: if has_router {
                DetectionConfidence::Exact
            } else {
                DetectionConfidence::Heuristic
            },
            signals,
            reasons: vec!["File-based routing framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // Collect all files under routes/ directories
        for routes_dir in &["src/routes", "app/routes"] {
            let dir = workspace_root.join(routes_dir);
            if dir.exists() {
                collect_files_recursive(&dir, &mut entries);
            }
        }

        // Generated route tree (TanStack Router convention)
        for name in &["src/routeTree.gen.ts", "src/routeTree.gen.tsx"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        // Router config files
        for name in &[
            "src/router.ts",
            "src/router.tsx",
            "src/router.js",
            "app/client.tsx",
            "app/server.tsx",
            "app/router.tsx",
            "app/ssr.tsx",
            "app/entry.client.tsx",
            "app/entry.server.tsx",
            "app/root.tsx",
        ] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        vec![("**/routeTree.gen.*".to_string(), FileClassification::Generated)]
    }

    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![FrameworkClassificationRule {
            pattern: "**/routeTree.gen.*".into(),
            classification: FileClassification::Generated,
        }]
    }
}

// ---------------------------------------------------------------------------

/// Generic root config file pack.
/// Picks up common config files that are consumed by build tools / plugins
/// but never explicitly imported in source code.
struct RootConfigPack;

impl FrameworkPack for RootConfigPack {
    fn name(&self) -> &'static str {
        "root-config"
    }

    fn detect(&self, _workspace_root: &Path, _manifest: &PackageManifest) -> bool {
        // Always active — config files are common to all JS/TS projects.
        true
    }

    fn detect_detailed(
        &self,
        _workspace_root: &Path,
        _manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        Some(FrameworkDetection {
            name: self.name(),
            confidence: DetectionConfidence::Exact,
            signals: vec!["always-on pack for root config files".into()],
            reasons: vec!["Root config files are common to all JS/TS projects".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // Well-known config files that are consumed by tooling, not imported
        let config_files = [
            "content-collections.ts",
            "content-collections.js",
            "tailwind.config.ts",
            "tailwind.config.js",
            "tailwind.config.mjs",
            "postcss.config.ts",
            "postcss.config.js",
            "postcss.config.cjs",
            "postcss.config.mjs",
            "drizzle.config.ts",
            "drizzle.config.js",
            "knexfile.ts",
            "knexfile.js",
            "tsup.config.ts",
            "tsup.config.js",
            "rollup.config.ts",
            "rollup.config.js",
            "rollup.config.mjs",
            "esbuild.config.ts",
            "esbuild.config.js",
            "webpack.config.ts",
            "webpack.config.js",
            "babel.config.ts",
            "babel.config.js",
            "release.config.ts",
            "release.config.js",
            "release.config.mts",
            "commitlint.config.ts",
            "commitlint.config.js",
            ".lintstagedrc.ts",
            ".lintstagedrc.js",
            "playwright.config.ts",
            "playwright.config.js",
            "cypress.config.ts",
            "cypress.config.js",
            "wrangler.config.ts",
            "wrangler.toml",
            // ESLint config files (flat config and legacy)
            "eslint.config.ts",
            "eslint.config.js",
            "eslint.config.mjs",
            "eslint.config.cjs",
            ".eslintrc.js",
            ".eslintrc.cjs",
            ".eslintrc.mjs",
            // Prettier config
            "prettier.config.ts",
            "prettier.config.js",
            "prettier.config.mjs",
            ".prettierrc.js",
            ".prettierrc.cjs",
            ".prettierrc.mjs",
            // Next.js i18n (next-intl/next-i18next)
            "i18n/request.ts",
            "i18n/request.js",
            // Sentry
            "sentry.client.config.ts",
            "sentry.server.config.ts",
            "sentry.edge.config.ts",
            "sentry.client.config.js",
            "sentry.server.config.js",
        ];

        for name in &config_files {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec![".content-collections/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }
}

// ===========================================================================
// Tier 1 — App Frameworks
// ===========================================================================

struct NuxtPack;

impl FrameworkPack for NuxtPack {
    fn name(&self) -> &'static str {
        "nuxt"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dep(manifest, "nuxt")
            || find_config_file(workspace_root, "nuxt.config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["nuxt"], &["nuxt.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Nuxt framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // nuxt.config.*
        collect_config_variants(workspace_root, "nuxt.config", &mut entries);

        // app.vue
        let app_vue = workspace_root.join("app.vue");
        if app_vue.exists() {
            entries.push(app_vue);
        }

        // File-based routing and convention directories
        for dir_name in &[
            "pages",
            "layouts",
            "middleware",
            "plugins",
            "server/api",
            "server/routes",
            "server/middleware",
        ] {
            let dir = workspace_root.join(dir_name);
            if dir.exists() {
                collect_files_recursive(&dir, &mut entries);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec![".nuxt/**".to_string(), ".output/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn auto_loaded_patterns(&self) -> Vec<String> {
        vec!["composables/**".to_string(), "utils/**".to_string()]
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec![".nuxt/**".to_string(), ".output/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct AstroPack;

impl FrameworkPack for AstroPack {
    fn name(&self) -> &'static str {
        "astro"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dep(manifest, "astro")
            || find_config_file(workspace_root, "astro.config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["astro"], &["astro.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Astro framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // astro.config.*
        collect_config_variants(workspace_root, "astro.config", &mut entries);

        // src/pages/**
        let pages_dir = workspace_root.join("src/pages");
        if pages_dir.exists() {
            collect_files_recursive(&pages_dir, &mut entries);
        }

        // src/middleware.*
        for ext in &["ts", "js", "mts", "mjs"] {
            let path = workspace_root.join(format!("src/middleware.{ext}"));
            if path.exists() {
                entries.push(path);
            }
        }

        // src/content/config.*
        for ext in &["ts", "js", "mts"] {
            let path = workspace_root.join(format!("src/content/config.{ext}"));
            if path.exists() {
                entries.push(path);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["dist/**".to_string(), ".astro/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["dist/**".to_string(), ".astro/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct SvelteKitPack;

impl FrameworkPack for SvelteKitPack {
    fn name(&self) -> &'static str {
        "sveltekit"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dep(manifest, "@sveltejs/kit")
            || find_config_file(workspace_root, "svelte.config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) = build_detection_signals(
            workspace_root,
            manifest,
            &["@sveltejs/kit"],
            &["svelte.config"],
        );

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["SvelteKit framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // svelte.config.*
        collect_config_variants(workspace_root, "svelte.config", &mut entries);

        // SvelteKit route files: +page.*, +layout.*, +server.*, +error.*
        let routes_dir = workspace_root.join("src/routes");
        if routes_dir.exists() {
            collect_sveltekit_route_files(&routes_dir, &mut entries);
        }

        // Hook files
        for name in &[
            "src/hooks.server.ts",
            "src/hooks.server.js",
            "src/hooks.client.ts",
            "src/hooks.client.js",
            "src/service-worker.ts",
            "src/service-worker.js",
        ] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec![".svelte-kit/**".to_string(), "build/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec![".svelte-kit/**".to_string(), "build/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct RemixPack;

impl FrameworkPack for RemixPack {
    fn name(&self) -> &'static str {
        "remix"
    }

    fn detect(&self, _workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_any_dep(manifest, &["@remix-run/dev", "remix"])
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, _has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["@remix-run/dev", "remix"], &[]);

        if !has_dep {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: DetectionConfidence::Exact,
            signals,
            reasons: vec!["Remix framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // app/root.*
        for ext in &["tsx", "ts", "jsx", "js"] {
            let path = workspace_root.join(format!("app/root.{ext}"));
            if path.exists() {
                entries.push(path);
            }
        }

        // app/routes/**
        let routes_dir = workspace_root.join("app/routes");
        if routes_dir.exists() {
            collect_files_recursive(&routes_dir, &mut entries);
        }

        // app/entry.client.* and app/entry.server.*
        for base in &["app/entry.client", "app/entry.server"] {
            for ext in &["tsx", "ts", "jsx", "js"] {
                let path = workspace_root.join(format!("{base}.{ext}"));
                if path.exists() {
                    entries.push(path);
                }
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["build/**".to_string(), ".cache/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["build/**".to_string(), ".cache/**".to_string()]
    }
}

// ===========================================================================
// Tier 2 — Monorepo / Build Systems
// ===========================================================================

struct NxPack;

impl FrameworkPack for NxPack {
    fn name(&self) -> &'static str {
        "nx"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_any_dep(manifest, &["nx"]) || workspace_root.join("nx.json").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let mut signals = Vec::new();
        let has_dep = manifest_has_any_dep(manifest, &["nx"]);
        if has_dep {
            signals.push("dependency `nx` found in package.json".into());
        }

        let has_config = workspace_root.join("nx.json").exists();
        if has_config {
            signals.push("`nx.json` found".into());
        }

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Nx monorepo tooling detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        let nx_json = workspace_root.join("nx.json");
        if nx_json.exists() {
            entries.push(nx_json);
        }

        // project.json at workspace root
        let project_json = workspace_root.join("project.json");
        if project_json.exists() {
            entries.push(project_json);
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["tmp/**".to_string(), ".nx/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn trust_notes(
        &self,
        workspace_root: &Path,
        _manifest: &PackageManifest,
    ) -> Vec<FrameworkTrustNote> {
        let mut notes = Vec::new();

        if workspace_root.join("nx.json").exists() {
            notes.push(FrameworkTrustNote {
                message: "Nx task graph configuration may contain dynamic targets that \
                          pruneguard cannot fully resolve; entrypoint detection is heuristic"
                    .into(),
                affects: TrustNoteScope::EntrypointsOnly,
            });
        }

        notes
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["tmp/**".to_string(), ".nx/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct TurboPack;

impl FrameworkPack for TurboPack {
    fn name(&self) -> &'static str {
        "turborepo"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "turbo") || workspace_root.join("turbo.json").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let mut signals = Vec::new();
        let has_dep = manifest_has_dev_dep(manifest, "turbo");
        if has_dep {
            signals.push("devDependency `turbo` found".into());
        }

        let has_config = workspace_root.join("turbo.json").exists();
        if has_config {
            signals.push("`turbo.json` found".into());
        }

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Turborepo monorepo tooling detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        let turbo_json = workspace_root.join("turbo.json");
        if turbo_json.exists() {
            entries.push(turbo_json);
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn trust_notes(
        &self,
        workspace_root: &Path,
        _manifest: &PackageManifest,
    ) -> Vec<FrameworkTrustNote> {
        let mut notes = Vec::new();

        if workspace_root.join("turbo.json").exists() {
            notes.push(FrameworkTrustNote {
                message: "Turborepo task graph awareness is heuristic; pipeline dependencies \
                          may not be fully resolved"
                    .into(),
                affects: TrustNoteScope::AllFindings,
            });
        }

        notes
    }
}

// ---------------------------------------------------------------------------

struct AngularPack;

impl FrameworkPack for AngularPack {
    fn name(&self) -> &'static str {
        "angular"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dep(manifest, "@angular/core") || workspace_root.join("angular.json").exists()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let mut signals = Vec::new();
        let has_dep = manifest_has_dep(manifest, "@angular/core");
        if has_dep {
            signals.push("dependency `@angular/core` found".into());
        }

        let has_config = workspace_root.join("angular.json").exists();
        if has_config {
            signals.push("`angular.json` found".into());
        }

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Angular framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        let angular_json = workspace_root.join("angular.json");
        if angular_json.exists() {
            entries.push(angular_json);
        }

        for name in &["src/main.ts", "src/polyfills.ts"] {
            let path = workspace_root.join(name);
            if path.exists() {
                entries.push(path);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["dist/**".to_string(), ".angular/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["dist/**".to_string(), ".angular/**".to_string()]
    }
}

// ===========================================================================
// Tier 3 — Dev / Runtime Tooling
// ===========================================================================

struct PlaywrightPack;

impl FrameworkPack for PlaywrightPack {
    fn name(&self) -> &'static str {
        "playwright"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "@playwright/test")
            || find_config_file(workspace_root, "playwright.config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) = build_detection_signals(
            workspace_root,
            manifest,
            &["@playwright/test"],
            &["playwright.config"],
        );

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Playwright end-to-end test framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // playwright.config.*
        collect_config_variants(workspace_root, "playwright.config", &mut entries);

        // e2e/**/*.spec.* and tests/**/*.spec.*
        for dir_name in &["e2e", "tests"] {
            let dir = workspace_root.join(dir_name);
            if dir.exists() {
                collect_spec_files_recursive(&dir, &mut entries);
            }
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        vec![("**/*.spec.*".to_string(), FileClassification::Test)]
    }

    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![FrameworkClassificationRule {
            pattern: "**/*.spec.*".into(),
            classification: FileClassification::Test,
        }]
    }
}

// ---------------------------------------------------------------------------

struct CypressPack;

impl FrameworkPack for CypressPack {
    fn name(&self) -> &'static str {
        "cypress"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "cypress")
            || find_config_file(workspace_root, "cypress.config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) =
            build_detection_signals(workspace_root, manifest, &["cypress"], &["cypress.config"]);

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Cypress end-to-end test framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // cypress.config.*
        collect_config_variants(workspace_root, "cypress.config", &mut entries);

        // cypress/support/*
        let support_dir = workspace_root.join("cypress/support");
        if support_dir.exists()
            && let Ok(read_dir) = std::fs::read_dir(&support_dir)
        {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.is_file() && pruneguard_fs::has_js_ts_extension(&path) {
                    entries.push(path);
                }
            }
        }

        // cypress/e2e/**
        let e2e_dir = workspace_root.join("cypress/e2e");
        if e2e_dir.exists() {
            collect_files_recursive(&e2e_dir, &mut entries);
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        vec![
            ("cypress/e2e/**".to_string(), FileClassification::Test),
            ("cypress/support/**".to_string(), FileClassification::Config),
        ]
    }

    fn classification_rules(&self) -> Vec<FrameworkClassificationRule> {
        vec![
            FrameworkClassificationRule {
                pattern: "cypress/e2e/**".into(),
                classification: FileClassification::Test,
            },
            FrameworkClassificationRule {
                pattern: "cypress/support/**".into(),
                classification: FileClassification::Config,
            },
        ]
    }
}

// ---------------------------------------------------------------------------

struct VitePressPack;

impl FrameworkPack for VitePressPack {
    fn name(&self) -> &'static str {
        "vitepress"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dev_dep(manifest, "vitepress")
            || find_config_file(workspace_root, ".vitepress/config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let mut signals = Vec::new();
        let has_dep = manifest_has_dev_dep(manifest, "vitepress");
        if has_dep {
            signals.push("devDependency `vitepress` found".into());
        }

        let has_config = find_config_file(workspace_root, ".vitepress/config").is_some();
        if has_config {
            signals.push("`.vitepress/config.*` found".into());
        }

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["VitePress documentation framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // .vitepress/config.*
        collect_config_variants(workspace_root, ".vitepress/config", &mut entries);

        // .vitepress/theme/**
        let theme_dir = workspace_root.join(".vitepress/theme");
        if theme_dir.exists() {
            collect_files_recursive(&theme_dir, &mut entries);
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec![".vitepress/dist/**".to_string(), ".vitepress/cache/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec![".vitepress/dist/**".to_string(), ".vitepress/cache/**".to_string()]
    }
}

// ---------------------------------------------------------------------------

struct DocusaurusPack;

impl FrameworkPack for DocusaurusPack {
    fn name(&self) -> &'static str {
        "docusaurus"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest_has_dep(manifest, "@docusaurus/core")
            || find_config_file(workspace_root, "docusaurus.config").is_some()
    }

    fn detect_detailed(
        &self,
        workspace_root: &Path,
        manifest: &PackageManifest,
    ) -> Option<FrameworkDetection> {
        let (has_dep, has_config, signals) = build_detection_signals(
            workspace_root,
            manifest,
            &["@docusaurus/core"],
            &["docusaurus.config"],
        );

        if !has_dep && !has_config {
            return None;
        }

        Some(FrameworkDetection {
            name: self.name(),
            confidence: detection_confidence(has_dep, has_config),
            signals,
            reasons: vec!["Docusaurus documentation framework detected".into()],
        })
    }

    fn entrypoints(&self, workspace_root: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();

        // docusaurus.config.*
        collect_config_variants(workspace_root, "docusaurus.config", &mut entries);

        // src/pages/**
        let pages_dir = workspace_root.join("src/pages");
        if pages_dir.exists() {
            collect_files_recursive(&pages_dir, &mut entries);
        }

        // src/theme/**
        let theme_dir = workspace_root.join("src/theme");
        if theme_dir.exists() {
            collect_files_recursive(&theme_dir, &mut entries);
        }

        entries
    }

    fn ignore_patterns(&self) -> Vec<String> {
        vec!["build/**".to_string(), ".docusaurus/**".to_string()]
    }

    fn file_kinds(&self) -> Vec<(String, FileClassification)> {
        Vec::new()
    }

    fn generated_output_patterns(&self) -> Vec<String> {
        vec!["build/**".to_string(), ".docusaurus/**".to_string()]
    }
}
