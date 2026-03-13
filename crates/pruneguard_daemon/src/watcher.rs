use std::path::{Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::{DebouncedEvent, Debouncer, new_debouncer};
use tokio::sync::mpsc;

/// Extensions relevant to pruneguard analysis.
const WATCHED_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

/// Config/metadata files that trigger a rebuild.
const WATCHED_FILENAMES: &[&str] = &[
    "package.json",
    "pruneguard.json",
    ".pruneguardrc.json",
    "CODEOWNERS",
    "turbo.json",
    "nx.json",
    "angular.json",
];

/// Prefixes for config files matched by starts-with.
const WATCHED_PREFIXES: &[&str] = &["tsconfig"];

/// Prefixes for framework config files (matched as `<prefix>.<ext>`).
const FRAMEWORK_CONFIG_PREFIXES: &[&str] = &[
    "next.config",
    "nuxt.config",
    "vite.config",
    "vitest.config",
    "svelte.config",
    "remix.config",
    "astro.config",
    "playwright.config",
    "cypress.config",
    "docusaurus.config",
];

/// A file-system watcher that debounces events and sends
/// relevant file paths over a tokio channel.
pub struct FileWatcher {
    /// The debouncer must be kept alive for notifications to flow.
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    /// Receiving end of the change channel.
    pub changes_rx: mpsc::UnboundedReceiver<Vec<PathBuf>>,
}

impl std::fmt::Debug for FileWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWatcher").finish_non_exhaustive()
    }
}

/// Errors from the file watcher.
#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
    #[error("failed to create file watcher: {0}")]
    Init(String),
    #[error("failed to watch path: {0}")]
    Watch(String),
}

impl FileWatcher {
    /// Start watching the given project root for relevant file changes.
    ///
    /// Debounced events are sent over the returned channel.
    pub fn start(project_root: &Path) -> Result<Self, WatcherError> {
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();

        let mut debouncer = new_debouncer(
            Duration::from_millis(200),
            move |result: Result<Vec<DebouncedEvent>, notify::Error>| {
                match result {
                    Ok(events) => {
                        let paths: Vec<PathBuf> = events
                            .into_iter()
                            .map(|e| e.path)
                            .filter(|p| is_relevant_path(p))
                            .collect();
                        if !paths.is_empty() {
                            // Ignore send errors — the receiver may have been dropped
                            // during shutdown.
                            let _ = changes_tx.send(paths);
                        }
                    }
                    Err(err) => {
                        tracing::warn!("file watcher error: {err}");
                    }
                }
            },
        )
        .map_err(|err| WatcherError::Init(err.to_string()))?;

        // Watch the project root recursively.
        debouncer
            .watcher()
            .watch(project_root, notify::RecursiveMode::Recursive)
            .map_err(|err| WatcherError::Watch(err.to_string()))?;

        tracing::info!("watching {} recursively for file changes", project_root.display());

        Ok(Self { _debouncer: debouncer, changes_rx })
    }
}

/// Check if a path is relevant to pruneguard analysis.
fn is_relevant_path(path: &Path) -> bool {
    // Skip paths inside node_modules or hidden directories.
    let path_str = path.to_string_lossy();
    if path_str.contains("node_modules") || path_str.contains("/.") || path_str.contains("\\.") {
        // Allow .pruneguard paths (for config changes).
        if !path_str.contains(".pruneguard") {
            return false;
        }
    }

    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    // Check exact filename matches.
    if WATCHED_FILENAMES.contains(&file_name) {
        return true;
    }

    // Check prefix matches (e.g. tsconfig.json, tsconfig.app.json).
    if WATCHED_PREFIXES.iter().any(|prefix| {
        file_name.starts_with(prefix)
            && Path::new(file_name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    }) {
        return true;
    }

    // Check framework config prefixes (e.g. next.config.js, vite.config.ts).
    if FRAMEWORK_CONFIG_PREFIXES.iter().any(|prefix| file_name.starts_with(prefix)) {
        return true;
    }

    // Check extension matches.
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return WATCHED_EXTENSIONS.contains(&ext);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn relevant_source_files() {
        assert!(is_relevant_path(Path::new("src/foo.ts")));
        assert!(is_relevant_path(Path::new("src/bar.tsx")));
        assert!(is_relevant_path(Path::new("lib/util.js")));
        assert!(is_relevant_path(Path::new("lib/util.jsx")));
        assert!(is_relevant_path(Path::new("lib/util.mjs")));
        assert!(is_relevant_path(Path::new("lib/util.cjs")));
        assert!(is_relevant_path(Path::new("lib/util.mts")));
        assert!(is_relevant_path(Path::new("lib/util.cts")));
    }

    #[test]
    fn relevant_config_files() {
        assert!(is_relevant_path(Path::new("package.json")));
        assert!(is_relevant_path(Path::new("apps/web/package.json")));
        assert!(is_relevant_path(Path::new("tsconfig.json")));
        assert!(is_relevant_path(Path::new("tsconfig.app.json")));
        assert!(is_relevant_path(Path::new("pruneguard.json")));
        assert!(is_relevant_path(Path::new(".pruneguardrc.json")));
        assert!(is_relevant_path(Path::new("CODEOWNERS")));
    }

    #[test]
    fn relevant_framework_config_files() {
        assert!(is_relevant_path(Path::new("next.config.js")));
        assert!(is_relevant_path(Path::new("next.config.mjs")));
        assert!(is_relevant_path(Path::new("next.config.ts")));
        assert!(is_relevant_path(Path::new("nuxt.config.ts")));
        assert!(is_relevant_path(Path::new("vite.config.ts")));
        assert!(is_relevant_path(Path::new("vitest.config.ts")));
        assert!(is_relevant_path(Path::new("svelte.config.js")));
        assert!(is_relevant_path(Path::new("remix.config.js")));
        assert!(is_relevant_path(Path::new("astro.config.mjs")));
        assert!(is_relevant_path(Path::new("playwright.config.ts")));
        assert!(is_relevant_path(Path::new("cypress.config.ts")));
        assert!(is_relevant_path(Path::new("turbo.json")));
        assert!(is_relevant_path(Path::new("nx.json")));
        assert!(is_relevant_path(Path::new("angular.json")));
    }

    #[test]
    fn pruneguardrc_in_absolute_path() {
        // .pruneguardrc.json contains "/." in absolute paths but should be
        // allowed through the hidden-directory filter via the .pruneguard
        // exception.
        assert!(is_relevant_path(Path::new("/home/user/project/.pruneguardrc.json")));
    }

    #[test]
    fn irrelevant_files() {
        assert!(!is_relevant_path(Path::new("README.md")));
        assert!(!is_relevant_path(Path::new("Cargo.toml")));
        assert!(!is_relevant_path(Path::new("node_modules/foo/index.js")));
        assert!(!is_relevant_path(Path::new(".git/objects/ab/1234")));
    }
}
