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
        // App router pages
        for ext in &["ts", "tsx", "js", "jsx"] {
            let page = workspace_root.join(format!("app/page.{ext}"));
            if page.exists() {
                entries.push(page);
            }
            let layout = workspace_root.join(format!("app/layout.{ext}"));
            if layout.exists() {
                entries.push(layout);
            }
        }
        // Pages router
        for ext in &["ts", "tsx", "js", "jsx"] {
            let index = workspace_root.join(format!("pages/index.{ext}"));
            if index.exists() {
                entries.push(index);
            }
        }
        // next.config
        for name in &["next.config.js", "next.config.mjs", "next.config.ts"] {
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
            for name in &["main.ts", "main.js", "preview.ts", "preview.js"] {
                let path = storybook_dir.join(name);
                if path.exists() {
                    entries.push(path);
                }
            }
        }
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
