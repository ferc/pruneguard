use std::path::{Path, PathBuf};

use oxgraph_config::EntrypointsConfig;
use oxgraph_manifest::PackageManifest;
use rustc_hash::FxHashSet;

/// A detected entrypoint.
#[derive(Debug, Clone)]
pub struct Entrypoint {
    /// Path to the entrypoint file.
    pub path: PathBuf,
    /// How this entrypoint was detected.
    pub kind: EntrypointSource,
    /// Which profile this entrypoint belongs to.
    pub profile: EntrypointProfile,
}

/// How an entrypoint was discovered.
#[derive(Debug, Clone, Copy)]
pub enum EntrypointSource {
    PackageMain,
    PackageBin,
    PackageExports,
    ExplicitConfig,
    FrameworkPack,
    Convention,
}

/// Which analysis profile the entrypoint belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointProfile {
    Production,
    Development,
    Both,
}

impl std::fmt::Display for Entrypoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?}, {:?})", self.path.display(), self.kind, self.profile,)
    }
}

/// Detect entrypoints for a workspace package.
pub fn detect_entrypoints(
    workspace_root: &Path,
    manifest: &PackageManifest,
    config: &EntrypointsConfig,
    framework_entries: &[PathBuf],
) -> Vec<Entrypoint> {
    let mut entrypoints = Vec::new();
    let mut seen = FxHashSet::default();

    // 1. Explicit config entries
    for pattern in &config.include {
        if let Ok(paths) = glob::glob(&workspace_root.join(pattern).display().to_string()) {
            for path in paths.flatten() {
                if seen.insert(path.clone()) {
                    entrypoints.push(Entrypoint {
                        path,
                        kind: EntrypointSource::ExplicitConfig,
                        profile: EntrypointProfile::Both,
                    });
                }
            }
        }
    }

    // 2. Auto-detect from package.json
    if config.auto {
        for file in manifest.entrypoint_files() {
            let path = workspace_root.join(&file);
            if path.exists() && seen.insert(path.clone()) {
                let kind = if file.contains("bin") {
                    EntrypointSource::PackageBin
                } else {
                    EntrypointSource::PackageMain
                };
                entrypoints.push(Entrypoint { path, kind, profile: EntrypointProfile::Production });
            }
        }

        // Convention-based: index files at root
        for candidate in &[
            "src/index.ts",
            "src/index.tsx",
            "src/index.js",
            "src/index.jsx",
            "src/main.ts",
            "src/main.tsx",
            "src/main.js",
            "src/main.jsx",
            "index.ts",
            "index.js",
        ] {
            let path = workspace_root.join(candidate);
            if path.exists() && seen.insert(path.clone()) {
                entrypoints.push(Entrypoint {
                    path,
                    kind: EntrypointSource::Convention,
                    profile: EntrypointProfile::Production,
                });
            }
        }
    }

    // 3. Framework-detected entries
    for path in framework_entries {
        if seen.insert(path.clone()) {
            entrypoints.push(Entrypoint {
                path: path.clone(),
                kind: EntrypointSource::FrameworkPack,
                profile: EntrypointProfile::Production,
            });
        }
    }

    entrypoints
}
