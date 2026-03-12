use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rustc_hash::{FxHashMap, FxHashSet};

/// Classification for a tracked repository file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Source,
    Test,
    Story,
    Config,
    Generated,
    BuildOutput,
}

/// A tracked file in the repository inventory.
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub workspace: Option<String>,
    pub package: Option<String>,
    pub kind: FileKind,
    pub ignored_reason: Option<String>,
}

/// Options for collecting project files.
#[derive(Debug, Clone, Default)]
pub struct FileCollectionOptions {
    pub ignore_patterns: Vec<String>,
    pub workspace_roots: FxHashMap<String, PathBuf>,
    pub package_names: FxHashMap<String, String>,
    pub extra_classifications: Vec<(String, FileKind)>,
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

/// Collect all JS/TS files under a directory.
#[allow(clippy::implicit_hasher)]
pub fn collect_source_files(
    root: &Path,
    ignore_patterns: &[String],
    extensions: &FxHashSet<String>,
) -> Vec<PathBuf> {
    walk_files(root, ignore_patterns)
        .into_iter()
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
                if extensions.is_empty() {
                    has_js_ts_extension(path)
                } else {
                    extensions.contains(ext) || extensions.contains(&format!(".{ext}"))
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
            let package = workspace
                .as_ref()
                .and_then(|name| options.package_names.get(name))
                .cloned();
            let kind = classify_file(&relative, &classification_patterns);

            Some(FileRecord {
                path,
                relative_path: relative,
                workspace,
                package,
                kind,
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

fn compile_classifiers(
    patterns: &[(String, FileKind)],
) -> Vec<(GlobSet, FileKind)> {
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

    has_js_ts_extension(relative_path)
        || matches!(file_name, "package.json" | "tsconfig.json" | "tsconfig.base.json")
        || matches!(extension, "json")
            && (file_name.contains("config") || file_name.contains("schema"))
}

fn workspace_for_path(
    path: &Path,
    workspace_roots: &FxHashMap<String, PathBuf>,
) -> Option<String> {
    workspace_roots
        .iter()
        .filter(|(_, root)| path.starts_with(root))
        .max_by_key(|(_, root)| root.components().count())
        .map(|(name, _)| name.clone())
}

fn classify_file(relative_path: &Path, extra_patterns: &[(GlobSet, FileKind)]) -> FileKind {
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
    {
        return FileKind::Generated;
    }

    if path.contains("/dist/")
        || path.contains("/build/")
        || path.contains("/coverage/")
        || path.contains("/storybook-static/")
        || path.contains("/.next/")
    {
        return FileKind::BuildOutput;
    }

    if path.contains("/__tests__/")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
    {
        return FileKind::Test;
    }

    if file_name.contains(".stories.") || path.contains("/.storybook/") {
        return FileKind::Story;
    }

    if matches!(file_name, "package.json" | "tsconfig.json" | "tsconfig.base.json")
        || file_name.contains(".config.")
    {
        return FileKind::Config;
    }

    FileKind::Source
}
