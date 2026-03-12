use std::path::{Path, PathBuf};

use oxgraph_config::{EntrypointsConfig, FrameworkToggle, FrameworksConfig};
use oxgraph_fs::has_js_ts_extension;
use oxgraph_frameworks::FrameworkPack;
use oxgraph_manifest::PackageManifest;
use rustc_hash::FxHashSet;

/// A detected entrypoint seed used to initialize graph reachability.
#[derive(Debug, Clone)]
pub struct EntrypointSeed {
    pub path: PathBuf,
    pub kind: EntrypointKind,
    pub profile: EntrypointProfile,
    pub workspace: Option<String>,
    pub source: String,
}

/// How an entrypoint was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointKind {
    PackageMain,
    PackageBin,
    PackageExports,
    ExplicitConfig,
    FrameworkPack,
    Convention,
    PackageScript,
}

/// Which analysis profile the entrypoint belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntrypointProfile {
    Production,
    Development,
    Both,
}

impl EntrypointProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Development => "development",
            Self::Both => "all",
        }
    }
}

impl EntrypointKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PackageMain => "package-main",
            Self::PackageBin => "package-bin",
            Self::PackageExports => "package-exports",
            Self::ExplicitConfig => "explicit-config",
            Self::FrameworkPack => "framework-pack",
            Self::Convention => "convention",
            Self::PackageScript => "package-script",
        }
    }
}

impl std::fmt::Display for EntrypointSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}, {}, {})",
            self.path.display(),
            self.kind.as_str(),
            self.profile.as_str(),
            self.source,
        )
    }
}

/// Detect entrypoints for a workspace package.
#[allow(clippy::too_many_lines)]
pub fn detect_entrypoints(
    workspace_name: Option<&str>,
    workspace_root: &Path,
    manifest: &PackageManifest,
    config: &EntrypointsConfig,
    frameworks_config: Option<&FrameworksConfig>,
    framework_packs: &[Box<dyn FrameworkPack>],
) -> Vec<EntrypointSeed> {
    let mut entrypoints = Vec::new();
    let mut seen = FxHashSet::default();

    for pattern in &config.include {
        if let Ok(paths) = glob::glob(&workspace_root.join(pattern).display().to_string()) {
            for path in paths.flatten() {
                push_entrypoint(
                    &mut entrypoints,
                    &mut seen,
                    path,
                    EntrypointKind::ExplicitConfig,
                    EntrypointProfile::Both,
                    workspace_name,
                    format!("config:{pattern}"),
                );
            }
        }
    }

    if config.auto {
        for file in manifest.entrypoint_files() {
            let path = workspace_root.join(&file);
            if !path.exists() {
                continue;
            }

            let (kind, profile) = if is_bin_entry(&file, manifest) {
                (EntrypointKind::PackageBin, EntrypointProfile::Production)
            } else if is_exports_entry(&file, manifest) {
                (EntrypointKind::PackageExports, EntrypointProfile::Production)
            } else {
                (EntrypointKind::PackageMain, EntrypointProfile::Production)
            };

            push_entrypoint(
                &mut entrypoints,
                &mut seen,
                path,
                kind,
                profile,
                workspace_name,
                format!("package:{file}"),
            );
        }

        for (script_name, command) in manifest.scripts.as_ref().into_iter().flat_map(|scripts| scripts.iter()) {
            let profile = script_profile(script_name);
            for candidate in extract_script_entrypoint_candidates(command) {
                let path = workspace_root.join(&candidate);
                if !path.exists() || !has_js_ts_extension(&path) {
                    continue;
                }

                push_entrypoint(
                    &mut entrypoints,
                    &mut seen,
                    path,
                    EntrypointKind::PackageScript,
                    profile,
                    workspace_name,
                    format!("package-script:{script_name}:{candidate}"),
                );
            }
        }

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
            if path.exists() {
                push_entrypoint(
                    &mut entrypoints,
                    &mut seen,
                    path,
                    EntrypointKind::Convention,
                    EntrypointProfile::Production,
                    workspace_name,
                    format!("convention:{candidate}"),
                );
            }
        }
    }

    for pack in framework_packs {
        if !framework_enabled(pack.name(), frameworks_config, workspace_root, manifest, pack.as_ref()) {
            continue;
        }
        let profile = framework_profile(pack.name());
        for path in pack.entrypoints(workspace_root) {
            push_entrypoint(
                &mut entrypoints,
                &mut seen,
                path,
                EntrypointKind::FrameworkPack,
                profile,
                workspace_name,
                format!("framework:{}", pack.name()),
            );
        }
    }

    entrypoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.source.cmp(&b.source)));
    entrypoints
}

fn push_entrypoint(
    entrypoints: &mut Vec<EntrypointSeed>,
    seen: &mut FxHashSet<PathBuf>,
    path: PathBuf,
    kind: EntrypointKind,
    profile: EntrypointProfile,
    workspace_name: Option<&str>,
    source: String,
) {
    if seen.insert(path.clone()) {
        entrypoints.push(EntrypointSeed {
            path,
            kind,
            profile,
            workspace: workspace_name.map(ToString::to_string),
            source,
        });
    }
}

fn framework_enabled(
    name: &str,
    config: Option<&FrameworksConfig>,
    workspace_root: &Path,
    manifest: &PackageManifest,
    pack: &dyn FrameworkPack,
) -> bool {
    let toggle = config.and_then(|frameworks| match name {
        "next" => frameworks.next,
        "vite" => frameworks.vite,
        "vitest" => frameworks.vitest,
        "jest" => frameworks.jest,
        "storybook" => frameworks.storybook,
        _ => None,
    });

    match toggle {
        Some(FrameworkToggle::Off) => false,
        Some(FrameworkToggle::On) => true,
        Some(FrameworkToggle::Auto) | None => pack.detect(workspace_root, manifest),
    }
}

fn framework_profile(name: &str) -> EntrypointProfile {
    match name {
        "vitest" | "jest" | "storybook" => EntrypointProfile::Development,
        _ => EntrypointProfile::Production,
    }
}

fn is_bin_entry(path: &str, manifest: &PackageManifest) -> bool {
    match &manifest.bin {
        Some(oxgraph_manifest::BinField::Single(bin)) => bin == path,
        Some(oxgraph_manifest::BinField::Map(map)) => map.values().any(|value| value == path),
        None => false,
    }
}

fn is_exports_entry(path: &str, manifest: &PackageManifest) -> bool {
    manifest
        .exports
        .as_ref()
        .and_then(|exports| exports.to_string().contains(path).then_some(()))
        .is_some()
}

fn script_profile(name: &str) -> EntrypointProfile {
    match name {
        "start" | "serve" | "prod" | "build" => EntrypointProfile::Production,
        "test" | "lint" | "dev" | "storybook" | "bench" => EntrypointProfile::Development,
        _ if name.starts_with("test:")
            || name.starts_with("dev:")
            || name.starts_with("lint:")
            || name.starts_with("storybook:")
            || name.starts_with("bench:") =>
        {
            EntrypointProfile::Development
        }
        _ if name.starts_with("start:")
            || name.starts_with("serve:")
            || name.starts_with("prod:")
            || name.starts_with("build:") =>
        {
            EntrypointProfile::Production
        }
        _ => EntrypointProfile::Both,
    }
}

fn extract_script_entrypoint_candidates(command: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = FxHashSet::default();

    for raw in command.split_whitespace() {
        let token = raw
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | ',' | ';' | '(' | ')'))
            .trim();
        if token.is_empty() || token.starts_with('-') || token.contains('$') {
            continue;
        }

        let path = Path::new(token);
        if !looks_like_script_path(token, path) {
            continue;
        }

        let normalized = path
            .components()
            .as_path()
            .to_string_lossy()
            .to_string();
        if seen.insert(normalized.clone()) {
            candidates.push(normalized);
        }
    }

    candidates
}

fn looks_like_script_path(token: &str, path: &Path) -> bool {
    has_js_ts_extension(path)
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("src/")
        || token.starts_with("scripts/")
        || token.starts_with("bin/")
        || token.starts_with("app/")
}
