use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// The language/format of a tracked source file.
///
/// Used to route extraction and determine how to parse a file's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Js,
    Jsx,
    Ts,
    Tsx,
    Mts,
    Cts,
    Dts,
    Vue,
    Svelte,
    Astro,
    Mdx,
    Css,
}

impl SourceKind {
    /// Determine the source kind from a file path extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension().and_then(|e| e.to_str())?;
        // Handle `.d.ts` / `.d.mts` / `.d.cts` first.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if Path::new(stem).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("d")) {
            match ext {
                "ts" | "mts" | "cts" => return Some(Self::Dts),
                _ => {}
            }
        }
        match ext {
            "js" | "mjs" | "cjs" => Some(Self::Js),
            "jsx" => Some(Self::Jsx),
            "ts" => Some(Self::Ts),
            "tsx" => Some(Self::Tsx),
            "mts" => Some(Self::Mts),
            "cts" => Some(Self::Cts),
            "vue" => Some(Self::Vue),
            "svelte" => Some(Self::Svelte),
            "astro" => Some(Self::Astro),
            "mdx" => Some(Self::Mdx),
            "css" | "scss" | "sass" | "less" => Some(Self::Css),
            _ => None,
        }
    }

    /// Whether this kind is a plain JS/TS variant (not a framework SFC).
    pub const fn is_js_ts(self) -> bool {
        matches!(
            self,
            Self::Js | Self::Jsx | Self::Ts | Self::Tsx | Self::Mts | Self::Cts | Self::Dts
        )
    }

    /// Whether this kind is a framework single-file component.
    pub const fn is_framework_sfc(self) -> bool {
        matches!(self, Self::Vue | Self::Svelte | Self::Astro | Self::Mdx)
    }

    /// Whether this kind is a stylesheet (CSS, SCSS, SASS, LESS).
    pub const fn is_css(self) -> bool {
        matches!(self, Self::Css)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Js => "js",
            Self::Jsx => "jsx",
            Self::Ts => "ts",
            Self::Tsx => "tsx",
            Self::Mts => "mts",
            Self::Cts => "cts",
            Self::Dts => "dts",
            Self::Vue => "vue",
            Self::Svelte => "svelte",
            Self::Astro => "astro",
            Self::Mdx => "mdx",
            Self::Css => "css",
        }
    }
}

/// Classification for a tracked repository file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Source,
    Test,
    Story,
    Config,
    Generated,
    BuildOutput,
}

/// Finer-grained classification used for entrypoint and analyzer semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileRole {
    Source,
    Test,
    Story,
    Fixture,
    Example,
    Template,
    Benchmark,
    Config,
    Generated,
    BuildOutput,
}

impl FileRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Test => "test",
            Self::Story => "story",
            Self::Fixture => "fixture",
            Self::Example => "example",
            Self::Template => "template",
            Self::Benchmark => "benchmark",
            Self::Config => "config",
            Self::Generated => "generated",
            Self::BuildOutput => "buildOutput",
        }
    }

    pub const fn kind(self) -> FileKind {
        match self {
            Self::Source | Self::Fixture | Self::Example | Self::Template | Self::Benchmark => {
                FileKind::Source
            }
            Self::Test => FileKind::Test,
            Self::Story => FileKind::Story,
            Self::Config => FileKind::Config,
            Self::Generated => FileKind::Generated,
            Self::BuildOutput => FileKind::BuildOutput,
        }
    }

    pub const fn is_development_only(self) -> bool {
        matches!(self, Self::Test | Self::Story)
    }

    pub const fn excluded_from_dead_code_by_default(self) -> bool {
        matches!(
            self,
            Self::Fixture
                | Self::Example
                | Self::Template
                | Self::Benchmark
                | Self::Config
                | Self::Generated
                | Self::BuildOutput
        )
    }

    pub const fn excluded_from_auto_entrypoints(self) -> bool {
        matches!(self, Self::Fixture | Self::Example | Self::Template | Self::Benchmark)
    }
}

/// A tracked file in the repository inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub workspace: Option<String>,
    pub package: Option<String>,
    pub kind: FileKind,
    pub role: FileRole,
    pub source_kind: Option<SourceKind>,
    pub ignored_reason: Option<String>,
}

/// Options for collecting project files.
#[derive(Debug, Clone, Default)]
pub struct FileCollectionOptions {
    pub ignore_patterns: Vec<String>,
    pub workspace_roots: FxHashMap<String, PathBuf>,
    pub package_names: FxHashMap<String, String>,
    pub extra_classifications: Vec<(String, FileRole)>,
}

/// Walk a directory tree, respecting `.gitignore` and custom ignore patterns.
pub fn walk_files(root: &Path, ignore_patterns: &[String]) -> Vec<PathBuf> {
    let ignore_set = compile_globset(ignore_patterns);
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);

    builder
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type()?.is_file() {
                return None;
            }

            let path = entry.into_path();
            let relative = path.strip_prefix(root).ok()?;
            if should_skip_path(relative) {
                return None;
            }

            if ignore_set.as_ref().is_some_and(|set| set.is_match(relative)) {
                return None;
            }

            Some(path)
        })
        .collect()
}

/// Check if a path matches any of the given extensions.
pub fn has_js_ts_extension(path: &Path) -> bool {
    static EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "tsx", "mts", "cts"];
    path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| EXTENSIONS.contains(&ext))
}

/// Check if a path is a tracked source file (JS/TS or framework SFC).
pub fn is_tracked_source(path: &Path) -> bool {
    SourceKind::from_path(path).is_some()
}

/// Collect all tracked source files under a directory.
pub fn collect_source_files(
    root: &Path,
    ignore_patterns: &[String],
    extensions: &[String],
) -> Vec<PathBuf> {
    walk_files(root, ignore_patterns)
        .into_iter()
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
                if extensions.is_empty() {
                    is_tracked_source(path)
                } else {
                    extensions.iter().any(|e| e == ext || e == &format!(".{ext}"))
                }
            })
        })
        .collect()
}

/// Collect all tracked project files and classify them.
pub fn collect_file_records(root: &Path, options: &FileCollectionOptions) -> Vec<FileRecord> {
    let classification_patterns = compile_classifiers(&options.extra_classifications);
    let mut records = walk_files(root, &options.ignore_patterns)
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(root).ok()?.to_path_buf();
            if !should_track_file(&relative) {
                return None;
            }

            let workspace = workspace_for_path(&path, &options.workspace_roots);
            let package =
                workspace.as_ref().and_then(|name| options.package_names.get(name)).cloned();
            let role = classify_file(&relative, &classification_patterns);
            let kind = role.kind();
            let source_kind = SourceKind::from_path(&relative);

            Some(FileRecord {
                path,
                relative_path: relative,
                workspace,
                package,
                kind,
                role,
                source_kind,
                ignored_reason: None,
            })
        })
        .collect::<Vec<_>>();

    records.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    records
}

fn compile_globset(patterns: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut added = false;
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
            added = true;
        }
    }

    if !added {
        return None;
    }

    builder.build().ok()
}

fn compile_classifiers(patterns: &[(String, FileRole)]) -> Vec<(GlobSet, FileRole)> {
    let mut matchers = Vec::new();
    for (pattern, kind) in patterns {
        let Some(globset) = compile_globset(std::slice::from_ref(pattern)) else {
            continue;
        };
        matchers.push((globset, *kind));
    }
    matchers
}

fn should_skip_path(relative_path: &Path) -> bool {
    relative_path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        matches!(value.as_ref(), ".git" | "node_modules" | "target")
    })
}

fn should_track_file(relative_path: &Path) -> bool {
    let file_name = relative_path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    let extension = relative_path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();

    is_tracked_source(relative_path)
        || matches!(file_name, "package.json" | "tsconfig.json" | "tsconfig.base.json")
        || matches!(extension, "json")
            && (file_name.contains("config") || file_name.contains("schema"))
}

fn workspace_for_path(path: &Path, workspace_roots: &FxHashMap<String, PathBuf>) -> Option<String> {
    workspace_roots
        .iter()
        .filter(|(_, root)| path.starts_with(root))
        .max_by_key(|(_, root)| root.components().count())
        .map(|(name, _)| name.clone())
}

pub fn is_docs_path(relative_path: &Path) -> bool {
    relative_path.components().next().is_some_and(|component| component.as_os_str() == "docs")
}

fn classify_file(relative_path: &Path, extra_patterns: &[(GlobSet, FileRole)]) -> FileRole {
    for (matcher, kind) in extra_patterns {
        if matcher.is_match(relative_path) {
            return *kind;
        }
    }

    let path = relative_path.to_string_lossy();
    let file_name = relative_path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

    if path.contains("/generated/")
        || path.contains("/__generated__/")
        || file_name.ends_with(".generated.ts")
        || file_name.ends_with(".generated.js")
        || file_name.ends_with(".gen.ts")
        || file_name.ends_with(".gen.tsx")
        || file_name.ends_with(".gen.js")
        || file_name.ends_with(".gen.jsx")
    {
        return FileRole::Generated;
    }

    if path.contains("/dist/")
        || path.contains("/build/")
        || path.contains("/coverage/")
        || path.contains("/storybook-static/")
        || path.contains("/.next/")
        || path.contains("/.nuxt/")
        || path.contains("/.output/")
        || path.contains("/.svelte-kit/")
        || path.contains("/.mastra/")
        || path.contains("/.turbo/")
        || path.contains("/.vercel/")
        || path.contains("/.astro/")
        || path.contains("/.angular/")
        || path.contains("/.cache/")
        || path.starts_with(".next/")
        || path.starts_with(".nuxt/")
        || path.starts_with(".output/")
        || path.starts_with(".svelte-kit/")
        || path.starts_with(".mastra/")
        || path.starts_with(".turbo/")
        || path.starts_with(".vercel/")
        || path.starts_with(".astro/")
        || path.starts_with(".angular/")
        || path.starts_with(".cache/")
    {
        return FileRole::BuildOutput;
    }

    if path.starts_with("fixtures/")
        || path.contains("/fixtures/")
        || path.contains("/test-fixtures/")
        || path.contains("/snapshots/")
        || path.contains("/test-files/")
        || file_name.ends_with(".snapshot.ts")
        || file_name.ends_with(".snapshot.tsx")
        || file_name.ends_with(".snapshot.js")
        || file_name.ends_with(".snapshot.jsx")
    {
        return FileRole::Fixture;
    }

    if path.starts_with("examples/") || path.contains("/examples/") {
        return FileRole::Example;
    }

    if path.starts_with("templates/") || path.contains("/templates/") {
        return FileRole::Template;
    }

    if path.starts_with("benchmarks/") || path.contains("/benchmarks/") {
        return FileRole::Benchmark;
    }

    if path.contains("/__tests__/")
        || path.starts_with("test/")
        || path.starts_with("tests/")
        || path.starts_with("e2e/")
        || path.contains("/e2e/")
        || path.contains("/__mocks__/")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
        || file_name.contains(".test-utils.")
    {
        return FileRole::Test;
    }

    if file_name.contains(".stories.")
        || file_name.contains(".story.")
        || path.contains("/.storybook/")
        || path.starts_with("stories/")
    {
        return FileRole::Story;
    }

    if matches!(file_name, "package.json" | "tsconfig.json" | "tsconfig.base.json")
        || file_name.contains(".config.")
    {
        return FileRole::Config;
    }

    FileRole::Source
}
