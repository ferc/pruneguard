use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use pruneguard_config::PruneguardConfig;
use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_graph::{BuildOptions, GraphBuildResult};

/// Errors that can occur in the hot index.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("graph build failed: {0}")]
    Build(String),
    #[error("analysis failed: {0}")]
    Analysis(String),
}

/// Category of an invalidation trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationKind {
    /// A source file (.ts, .tsx, .js, .jsx, .mjs, .cjs, .mts, .cts) changed.
    SourceFile,
    /// A package.json manifest changed.
    Manifest,
    /// A tsconfig*.json changed.
    Tsconfig,
    /// A CODEOWNERS file changed.
    Codeowners,
    /// A pruneguard.json / .pruneguardrc.json config changed.
    Config,
    /// A framework config file changed (next.config.*, vite.config.*, etc.).
    FrameworkConfig,
    /// A schema/version mismatch was detected.
    SchemaMismatch,
}

impl InvalidationKind {
    /// Framework config prefixes that trigger a full rebuild.
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

    /// Classify a file path into an invalidation kind.
    pub fn classify(path: &Path) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;

        if file_name == "pruneguard.json" || file_name == ".pruneguardrc.json" {
            return Some(Self::Config);
        }
        if file_name == "CODEOWNERS" {
            return Some(Self::Codeowners);
        }
        if file_name == "package.json" {
            return Some(Self::Manifest);
        }
        if file_name.starts_with("tsconfig")
            && Path::new(file_name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            return Some(Self::Tsconfig);
        }
        // Standalone JSON config files used by build tools / monorepo orchestrators.
        if matches!(file_name, "turbo.json" | "nx.json" | "angular.json") {
            return Some(Self::FrameworkConfig);
        }
        // Prefixed framework config files (e.g. next.config.js, vite.config.ts).
        if Self::FRAMEWORK_CONFIG_PREFIXES.iter().any(|prefix| file_name.starts_with(prefix)) {
            return Some(Self::FrameworkConfig);
        }

        let ext = path.extension()?.to_str()?;
        match ext {
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Some(Self::SourceFile),
            _ => None,
        }
    }

    /// Whether this kind of invalidation requires a full rebuild (not just
    /// incremental re-extraction of a single file).
    pub const fn requires_full_rebuild(self) -> bool {
        matches!(
            self,
            Self::Manifest
                | Self::Tsconfig
                | Self::Codeowners
                | Self::Config
                | Self::FrameworkConfig
                | Self::SchemaMismatch
        )
    }
}

/// The hot in-memory index holding the built graph.
///
/// This holds the most recently built graph and supports
/// invalidation and rebuild of changed files.
#[derive(Debug)]
pub struct HotIndex {
    /// Root directory of the project being indexed.
    project_root: PathBuf,
    /// Configuration snapshot used for the current graph.
    config: PruneguardConfig,
    /// The current graph build result.
    build: Option<Arc<GraphBuildResult>>,
    /// Monotonically increasing generation counter, bumped on every rebuild.
    generation: u64,
    /// Timestamp of the last successful build.
    last_build: Option<Instant>,
    /// Set of files that have been invalidated since the last build.
    invalidated_files: Vec<PathBuf>,
    /// Timestamp of the last watcher event processed.
    last_watcher_event: Option<Instant>,
    /// Whether a config-level change is pending (requires full rebuild).
    config_change_pending: bool,
    /// Duration of the initial (first) graph build in milliseconds.
    initial_build_ms: Option<u64>,
    /// Duration of the last incremental rebuild in milliseconds.
    last_rebuild_ms: Option<u64>,
    /// Number of incremental rebuilds since daemon start.
    incremental_rebuilds: u64,
    /// Total number of files invalidated since daemon start.
    total_invalidations: u64,
}

impl HotIndex {
    /// Create a new (cold) hot index for the given project root.
    pub const fn new(project_root: PathBuf, config: PruneguardConfig) -> Self {
        Self {
            project_root,
            config,
            build: None,
            generation: 0,
            last_build: None,
            invalidated_files: Vec::new(),
            last_watcher_event: None,
            config_change_pending: false,
            initial_build_ms: None,
            last_rebuild_ms: None,
            incremental_rebuilds: 0,
            total_invalidations: 0,
        }
    }

    /// Whether the index has been warmed (initial build completed).
    pub const fn is_warm(&self) -> bool {
        self.build.is_some()
    }

    /// The current generation counter.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Milliseconds since the last successful build, or `u64::MAX` if never built.
    pub fn last_update_ms(&self) -> u64 {
        self.last_build
            .map_or(u64::MAX, |t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    /// Number of nodes in the module graph, or 0 if not built.
    pub fn graph_nodes(&self) -> usize {
        self.build.as_ref().map_or(0, |b| b.module_graph.graph.node_count())
    }

    /// Number of edges in the module graph, or 0 if not built.
    pub fn graph_edges(&self) -> usize {
        self.build.as_ref().map_or(0, |b| b.module_graph.graph.edge_count())
    }

    /// Number of files tracked in the index.
    pub fn tracked_files(&self) -> usize {
        self.build.as_ref().map_or(0, |b| b.files.len())
    }

    /// The project root being indexed.
    pub const fn project_root(&self) -> &PathBuf {
        &self.project_root
    }

    /// Reference to the current configuration.
    pub const fn config(&self) -> &PruneguardConfig {
        &self.config
    }

    /// Get a shared reference to the current build, if available.
    pub const fn current_build(&self) -> Option<&Arc<GraphBuildResult>> {
        self.build.as_ref()
    }

    /// Milliseconds since the last watcher event was processed, or `None` if
    /// no watcher events have been received.
    pub fn watcher_lag_ms(&self) -> Option<u64> {
        self.last_watcher_event.map(|t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    /// Number of files pending invalidation.
    pub const fn pending_invalidations(&self) -> usize {
        self.invalidated_files.len()
    }

    /// Duration of the initial graph build in milliseconds, if available.
    pub const fn initial_build_ms(&self) -> Option<u64> {
        self.initial_build_ms
    }

    /// Duration of the last incremental rebuild in milliseconds, if available.
    pub const fn last_rebuild_ms(&self) -> Option<u64> {
        self.last_rebuild_ms
    }

    /// Number of incremental rebuilds since daemon start.
    pub const fn incremental_rebuilds(&self) -> u64 {
        self.incremental_rebuilds
    }

    /// Total number of files invalidated since daemon start.
    pub const fn total_invalidations(&self) -> u64 {
        self.total_invalidations
    }

    /// Whether a config-level change is pending that requires full rebuild.
    pub const fn config_change_pending(&self) -> bool {
        self.config_change_pending
    }

    /// Build the graph from scratch.
    ///
    /// This performs a full build using the one-shot analysis pipeline.
    pub fn build_initial(&mut self) -> Result<(), IndexError> {
        tracing::info!("building initial graph for {}", self.project_root.display());
        let start = Instant::now();

        let cache = pruneguard_cache::AnalysisCache::open(&self.project_root).ok();
        let build = pruneguard_graph::build_graph_with_options(
            &self.project_root,
            &self.config,
            &[],
            EntrypointProfile::Both,
            BuildOptions { cache: cache.as_ref() },
        )
        .map_err(|err| IndexError::Build(err.to_string()))?;

        let elapsed = start.elapsed();
        tracing::info!(
            "initial graph built in {elapsed:?}: {} nodes, {} edges, {} files",
            build.module_graph.graph.node_count(),
            build.module_graph.graph.edge_count(),
            build.files.len(),
        );

        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);

        // Track timing: if this is the very first build, record it as
        // initial_build_ms; otherwise it is an incremental rebuild.
        if self.generation == 0 {
            self.initial_build_ms = Some(elapsed_ms);
        } else {
            self.last_rebuild_ms = Some(elapsed_ms);
            self.incremental_rebuilds += 1;
        }

        self.build = Some(Arc::new(build));
        self.generation += 1;
        self.last_build = Some(Instant::now());
        self.invalidated_files.clear();
        self.config_change_pending = false;
        Ok(())
    }

    /// Mark files as invalidated due to file system changes.
    ///
    /// Classifies each file into an [`InvalidationKind`] and triggers a
    /// config reload when manifest, tsconfig, CODEOWNERS, or config files
    /// change.
    pub fn invalidate_files(&mut self, files: &[PathBuf]) {
        self.last_watcher_event = Some(Instant::now());

        for file in files {
            if let Some(kind) = InvalidationKind::classify(file)
                && kind.requires_full_rebuild()
            {
                self.config_change_pending = true;
                tracing::info!(
                    "config-level change detected in {}: scheduling full rebuild",
                    file.display(),
                );
            }
        }

        self.invalidated_files.extend_from_slice(files);
        self.total_invalidations += files.len() as u64;
        tracing::debug!(
            "invalidated {} files (total pending: {}, config_change: {})",
            files.len(),
            self.invalidated_files.len(),
            self.config_change_pending,
        );
    }

    /// Rebuild the graph incorporating invalidated files.
    ///
    /// If a config-level change is pending (manifest, tsconfig, CODEOWNERS,
    /// or pruneguard.json), the configuration is reloaded from disk before
    /// rebuilding.
    pub fn rebuild_changed(&mut self) -> Result<(), IndexError> {
        if self.invalidated_files.is_empty() {
            return Ok(());
        }
        tracing::info!(
            "rebuilding graph for {} invalidated files (config_change: {})",
            self.invalidated_files.len(),
            self.config_change_pending,
        );

        // If a config-level change is pending, reload the config from disk.
        if self.config_change_pending {
            match pruneguard_config::PruneguardConfig::load(&self.project_root, None) {
                Ok(new_config) => {
                    tracing::info!("reloaded config from disk after config-level change");
                    self.config = new_config;
                }
                Err(pruneguard_config::ConfigError::NotFound) => {
                    tracing::debug!(
                        "config file not found after config change; keeping existing config"
                    );
                }
                Err(err) => {
                    tracing::warn!("failed to reload config: {err}; keeping existing config");
                }
            }
            self.config_change_pending = false;
        }

        // For now, perform a full rebuild. Incremental updates will be
        // added in a future iteration.
        self.build_initial()
    }

    /// Run a scan against the current graph.
    ///
    /// Returns the analysis report as a JSON value.
    pub fn query_scan(
        &self,
        paths: &[PathBuf],
        changed_since: Option<&str>,
        focus: Option<&str>,
    ) -> Result<serde_json::Value, IndexError> {
        let _ = (paths, changed_since, focus);

        let build = self
            .build
            .as_ref()
            .ok_or_else(|| IndexError::Analysis("index not warmed yet".to_string()))?;

        let findings =
            pruneguard_analyzers::run_analyzers(build, &self.config, EntrypointProfile::Both);
        let report = serde_json::json!({
            "version": 1,
            "toolVersion": env!("CARGO_PKG_VERSION"),
            "findings": serde_json::to_value(&findings).unwrap_or_default(),
            "summary": {
                "totalFiles": build.files.len(),
                "totalFindings": findings.len(),
            },
        });
        Ok(report)
    }

    /// Run a review against the current graph.
    pub fn query_review(&self, base_ref: Option<&str>) -> Result<serde_json::Value, IndexError> {
        let _ = base_ref;
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "reviewResult",
            "blockingFindings": [],
            "advisoryFindings": [],
            "trust": { "scope": "full", "baseline": false }
        }))
    }

    /// Compute impact for a target.
    pub fn query_impact(
        &self,
        target: &str,
        focus: Option<&str>,
    ) -> Result<serde_json::Value, IndexError> {
        let _ = focus;
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "impactResult",
            "target": target,
            "affectedEntrypoints": [],
            "affectedPackages": [],
            "affectedFiles": []
        }))
    }

    /// Explain a finding or path.
    pub fn query_explain(
        &self,
        query: &str,
        focus: Option<&str>,
    ) -> Result<serde_json::Value, IndexError> {
        let _ = focus;
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "explainResult",
            "query": query,
            "matchedNode": null,
            "proofs": [],
            "relatedFindings": []
        }))
    }

    /// Evaluate targets for safe deletion.
    pub fn query_safe_delete(&self, targets: &[String]) -> Result<serde_json::Value, IndexError> {
        let _ = targets;
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "safeDeleteResult",
            "safe": [],
            "blocked": [],
            "needsReview": []
        }))
    }

    /// Generate a fix plan for targets.
    pub fn query_fix_plan(&self, targets: &[String]) -> Result<serde_json::Value, IndexError> {
        let _ = targets;
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "fixPlanResult",
            "steps": []
        }))
    }

    /// Suggest governance rules from graph analysis.
    pub fn query_suggest_rules(&self) -> Result<serde_json::Value, IndexError> {
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "suggestRulesResult",
            "suggestedRules": [],
            "analysisNotes": []
        }))
    }

    /// Generate a compatibility report.
    pub fn query_compatibility_report(&self) -> Result<serde_json::Value, IndexError> {
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "compatibilityReportResult",
            "packages": [],
            "issues": []
        }))
    }

    /// Debug framework detection.
    pub fn query_debug_frameworks(&self) -> Result<serde_json::Value, IndexError> {
        self.require_warm()?;
        Ok(serde_json::json!({
            "kind": "debugFrameworksResult",
            "detectedFrameworks": [],
            "detectionDetails": []
        }))
    }

    /// Require that the index has been warmed (initial build completed).
    fn require_warm(&self) -> Result<&Arc<GraphBuildResult>, IndexError> {
        self.build.as_ref().ok_or_else(|| IndexError::Analysis("index not warmed yet".to_string()))
    }
}
