use std::path::{Path, PathBuf};

use pruneguard_config::{EntrypointsConfig, FrameworkToggle, FrameworksConfig};
use pruneguard_frameworks::FrameworkPack;
use pruneguard_fs::{has_js_ts_extension, is_tracked_source};
use pruneguard_manifest::PackageManifest;
use rustc_hash::FxHashSet;

/// A detected entrypoint seed used to initialize graph reachability.
#[derive(Debug, Clone)]
pub struct EntrypointSeed {
    pub path: PathBuf,
    pub kind: EntrypointKind,
    pub surface_kind: EntrypointSurfaceKind,
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

/// What surface category an entrypoint belongs to.
///
/// Derived from `EntrypointKind` and the framework pack name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntrypointSurfaceKind {
    /// Public package API: main, bin, exports.
    PublicApi,
    /// General runtime entrypoints: explicit config, conventions, scripts.
    Runtime,
    /// Tooling-related entrypoints (linters, bundler config, etc.).
    Tooling,
    /// Test entrypoints (vitest, jest, etc.).
    Test,
    /// Story/doc entrypoints (storybook, etc.).
    Story,
    /// Framework-convention entrypoints (Next.js pages, Nuxt routes, etc.).
    FrameworkConvention,
}

impl EntrypointSurfaceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PublicApi => "public-api",
            Self::Runtime => "runtime",
            Self::Tooling => "tooling",
            Self::Test => "test",
            Self::Story => "story",
            Self::FrameworkConvention => "framework-convention",
        }
    }
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

/// Derive the surface kind from an `EntrypointKind` and the source string.
///
/// The `source` string carries framework information (e.g. `"framework:vitest"`).
fn derive_surface_kind(kind: EntrypointKind, source: &str) -> EntrypointSurfaceKind {
    match kind {
        EntrypointKind::PackageMain
        | EntrypointKind::PackageBin
        | EntrypointKind::PackageExports => EntrypointSurfaceKind::PublicApi,
        EntrypointKind::ExplicitConfig => EntrypointSurfaceKind::Runtime,
        EntrypointKind::Convention | EntrypointKind::PackageScript => {
            EntrypointSurfaceKind::Runtime
        }
        EntrypointKind::FrameworkPack => {
            // Determine more specific surface kind from the framework name
            // embedded in the source string (e.g. "framework:vitest", "framework:storybook").
            let fw_name = source
                .strip_prefix("framework:")
                .or_else(|| source.strip_prefix("framework-auto-load:"))
                .and_then(|rest| rest.split(':').next())
                .unwrap_or("");
            match fw_name {
                "vitest" | "jest" | "playwright" | "playwright-ct" | "playwright-test" | "cypress" => EntrypointSurfaceKind::Test,
                "storybook" => EntrypointSurfaceKind::Story,
                _ => EntrypointSurfaceKind::FrameworkConvention,
            }
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
            // Prefer source files over dist files.  If the entrypoint points to
            // a build-output directory (dist/, build/, etc.) try to find the
            // corresponding source file first.
            let path = resolve_dist_to_source(workspace_root, &file)
                .or_else(|| {
                    let p = workspace_root.join(&file);
                    p.exists().then_some(p)
                })
                .unwrap_or_else(|| workspace_root.join(&file));
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

        for (script_name, command) in
            manifest.scripts.as_ref().into_iter().flat_map(|scripts| scripts.iter())
        {
            let profile = script_profile(script_name);
            for candidate in extract_script_entrypoint_candidates(command) {
                let path = workspace_root.join(&candidate);
                if !path.exists() || !is_tracked_source(&path) {
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
        if !framework_enabled(
            pack.name(),
            frameworks_config,
            workspace_root,
            manifest,
            pack.as_ref(),
        ) {
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

        // Auto-loaded patterns: files the framework auto-imports at runtime
        // (e.g. Nuxt composables/, utils/, plugins/).
        for pattern in pack.auto_loaded_patterns() {
            // The Rust `glob` crate's `**` matches directory components but
            // not leaf files.  Ensure patterns that end with `/**` also have a
            // trailing wildcard to capture files (e.g. `composables/**` →
            // `composables/**/*`).
            let adjusted =
                if pattern.ends_with("/**") { format!("{pattern}/*") } else { pattern.clone() };
            let glob_pattern = workspace_root.join(&adjusted).display().to_string();
            if let Ok(paths) = glob::glob(&glob_pattern) {
                for path in paths.flatten() {
                    if is_tracked_source(&path) {
                        push_entrypoint(
                            &mut entrypoints,
                            &mut seen,
                            path,
                            EntrypointKind::FrameworkPack,
                            profile,
                            workspace_name,
                            format!("framework-auto-load:{}:{pattern}", pack.name()),
                        );
                    }
                }
            }
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
        let surface_kind = derive_surface_kind(kind, &source);
        entrypoints.push(EntrypointSeed {
            path,
            kind,
            surface_kind,
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
        "nuxt" => frameworks.nuxt,
        "astro" => frameworks.astro,
        "sveltekit" => frameworks.sveltekit,
        "remix" => frameworks.remix,
        "angular" => frameworks.angular,
        "nx" => frameworks.nx,
        "turborepo" => frameworks.turborepo,
        "playwright" => frameworks.playwright,
        "cypress" => frameworks.cypress,
        "vitepress" => frameworks.vitepress,
        "docusaurus" => frameworks.docusaurus,
        "vue" => frameworks.vue,
        "svelte" => frameworks.svelte,
        "babel" => frameworks.babel,
        "tanstack-router" => frameworks.tanstack_router,
        "vike" => frameworks.vike,
        "rslib" => frameworks.rslib,
        "playwright-ct" => frameworks.playwright_ct,
        "playwright-test" => frameworks.playwright_test,
        "nitro" => frameworks.nitro,
        "react-router" => frameworks.react_router,
        "rsbuild" => frameworks.rsbuild,
        "parcel" => frameworks.parcel,
        "qwik" => frameworks.qwik,
        "trigger-dev" => frameworks.trigger_dev,
        _ => None, // generic packs (file-routing, root-config) are auto-detect only
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
        Some(pruneguard_manifest::BinField::Single(bin)) => bin == path,
        Some(pruneguard_manifest::BinField::Map(map)) => map.values().any(|value| value == path),
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

    // Split on shell operators first to handle chained commands.
    for segment in command.split(&['&', '|', ';'][..]) {
        let segment = segment.trim();
        let mut tokens = segment.split_whitespace().peekable();
        let mut skip_next_path = false;

        // Detect runner prefix: `node`, `tsx`, `ts-node`, `pnpm exec`, etc.
        // After a runner, the first non-flag token is the script path.
        if let Some(first) = tokens.peek().copied() {
            match first {
                "node" | "tsx" | "ts-node" | "tsm" | "bun" => {
                    tokens.next(); // consume the runner
                    // Skip flags but capture file arguments after `-r`, `--require`, etc.
                    while let Some(token) = tokens.next() {
                        if matches!(token, "-r" | "--require" | "--loader" | "--import") {
                            let _ = tokens.next(); // skip the module argument
                            continue;
                        }
                        if token.starts_with('-') {
                            // Some flags take a value.
                            if token.starts_with("--require=") || token.starts_with("-r=") {
                                continue;
                            }
                            if matches!(token, "-e" | "--eval" | "-p" | "--print") {
                                skip_next_path = true;
                                break;
                            }
                            continue;
                        }
                        // This is the script path.
                        try_add_candidate(token, &mut candidates, &mut seen);
                        break;
                    }
                    if skip_next_path {
                        continue;
                    }
                    // Also check remaining tokens for additional file arguments.
                }
                "npx" | "bunx" => {
                    tokens.next(); // consume the runner
                    // Skip flags to find the package name, then look for file args after.
                    let mut found_pkg = false;
                    for token in tokens.by_ref() {
                        if token.starts_with('-') {
                            continue;
                        }
                        if !found_pkg {
                            found_pkg = true;
                            // The package name itself is not a script path.
                            continue;
                        }
                        try_add_candidate(token, &mut candidates, &mut seen);
                    }
                    continue;
                }
                "pnpm" | "yarn" | "npm" => {
                    tokens.next(); // consume the runner
                    // These might be `pnpm exec tsx script.ts` or `pnpm run build`.
                    // Skip workspace/filter flags.
                    // Note: we need `while let` (not `for`) because we call
                    // `tokens.next()` to consume flag values inside the loop.
                    #[allow(clippy::while_let_on_iterator)]
                    while let Some(token) = tokens.next() {
                        if token.starts_with('-') {
                            if matches!(
                                token,
                                "--filter" | "-F" | "--workspace" | "-w" | "--cwd" | "--prefix"
                            ) {
                                let _ = tokens.next();
                            }
                            continue;
                        }
                        // This is the subcommand (exec, run, dlx, or a package binary).
                        if matches!(token, "exec" | "dlx" | "x") {
                            // Next tokens are the command + args — recurse into them.
                            let remaining: Vec<&str> = tokens.collect();
                            let sub_cmd = remaining.join(" ");
                            let sub_candidates = extract_script_entrypoint_candidates(&sub_cmd);
                            for c in sub_candidates {
                                if seen.insert(c.clone()) {
                                    candidates.push(c);
                                }
                            }
                        }
                        // `pnpm run <script-name>` — no file path to extract.
                        break;
                    }
                    continue;
                }
                _ => {}
            }
        }

        // Generic token scanning for the remaining tokens.
        for raw in tokens {
            let token = raw
                .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | ',' | ';' | '(' | ')'))
                .trim();
            try_add_candidate(token, &mut candidates, &mut seen);
        }
    }

    candidates
}

fn try_add_candidate(token: &str, candidates: &mut Vec<String>, seen: &mut FxHashSet<String>) {
    if token.is_empty() || token.starts_with('-') || token.contains('$') {
        return;
    }

    let path = Path::new(token);
    if !looks_like_script_path(token, path) {
        return;
    }

    let normalized = path.components().as_path().to_string_lossy().to_string();
    if seen.insert(normalized.clone()) {
        candidates.push(normalized);
    }
}

fn looks_like_script_path(token: &str, path: &Path) -> bool {
    has_js_ts_extension(path)
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("src/")
        || token.starts_with("scripts/")
        || token.starts_with("bin/")
        || token.starts_with("app/")
        || token.starts_with("lib/")
        || token.starts_with("tools/")
}

/// When a bin/main/exports entry points to a build output (e.g. `./dist/bin.js`),
/// try to find the corresponding source file in `src/`.
fn resolve_dist_to_source(workspace_root: &Path, file: &str) -> Option<PathBuf> {
    // Strip leading "./"
    let file = file.strip_prefix("./").unwrap_or(file);

    // Only attempt for common build output directories.
    let stem = file
        .strip_prefix("dist/")
        .or_else(|| file.strip_prefix("build/"))
        .or_else(|| file.strip_prefix("out/"))
        .or_else(|| file.strip_prefix("lib/"))?;

    // Remove the JS extension to get the bare stem
    let bare = Path::new(stem);
    let bare_stem = bare.with_extension("");

    // Try source extensions under src/
    for ext in &["ts", "tsx", "js", "jsx", "mts", "cts"] {
        let candidate = workspace_root.join("src").join(bare_stem.with_extension(ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Also try the exact same relative path under src/ (e.g. dist/index.js -> src/index.ts)
    for ext in &["ts", "tsx", "js", "jsx"] {
        let candidate = workspace_root.join(bare_stem.with_extension(ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}
