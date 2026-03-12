use std::path::{Path, PathBuf};

use pruneguard_manifest::PackageManifest;

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
}

/// How a framework classifies a file.
#[derive(Debug, Clone, Copy)]
pub enum FileClassification {
    Test,
    Story,
    Config,
    Generated,
}

/// Get all built-in framework packs.
pub fn built_in_packs() -> Vec<Box<dyn FrameworkPack>> {
    vec![
        Box::new(NextPack),
        Box::new(VitePack),
        Box::new(VitestPack),
        Box::new(JestPack),
        Box::new(StorybookPack),
        Box::new(FileBasedRoutingPack),
        Box::new(RootConfigPack),
    ]
}

// --- Built-in packs ---

struct NextPack;

impl FrameworkPack for NextPack {
    fn name(&self) -> &'static str {
        "next"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest.dependencies.as_ref().is_some_and(|d| d.contains_key("next"))
            || manifest.dev_dependencies.as_ref().is_some_and(|d| d.contains_key("next"))
            || workspace_root.join("next.config.js").exists()
            || workspace_root.join("next.config.mjs").exists()
            || workspace_root.join("next.config.ts").exists()
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
        for name in &["instrumentation.ts", "instrumentation.js", "instrumentation-client.ts", "instrumentation-client.js", "src/instrumentation.ts", "src/instrumentation.js"] {
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
}

struct VitePack;

impl FrameworkPack for VitePack {
    fn name(&self) -> &'static str {
        "vite"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest.dev_dependencies.as_ref().is_some_and(|d| d.contains_key("vite"))
            || workspace_root.join("vite.config.ts").exists()
            || workspace_root.join("vite.config.js").exists()
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
}

struct VitestPack;

impl FrameworkPack for VitestPack {
    fn name(&self) -> &'static str {
        "vitest"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest.dev_dependencies.as_ref().is_some_and(|d| d.contains_key("vitest"))
            || workspace_root.join("vitest.config.ts").exists()
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
}

struct JestPack;

impl FrameworkPack for JestPack {
    fn name(&self) -> &'static str {
        "jest"
    }

    fn detect(&self, workspace_root: &Path, manifest: &PackageManifest) -> bool {
        manifest.dev_dependencies.as_ref().is_some_and(|d| d.contains_key("jest"))
            || workspace_root.join("jest.config.js").exists()
            || workspace_root.join("jest.config.ts").exists()
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
}

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
}

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

fn manifest_has_any_dep(manifest: &PackageManifest, deps: &[&str]) -> bool {
    let check =
        |d: &rustc_hash::FxHashMap<String, String>| deps.iter().any(|dep| d.contains_key(*dep));
    manifest.dependencies.as_ref().is_some_and(check)
        || manifest.dev_dependencies.as_ref().is_some_and(check)
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
