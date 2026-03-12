use std::path::{Path, PathBuf};

use rustc_hash::FxHashSet;

/// Walk a directory tree, respecting `.gitignore` and custom ignore patterns.
pub fn walk_files(root: &Path, ignore_patterns: &[String]) -> Vec<PathBuf> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(true).git_ignore(true).git_global(true);

    for pattern in ignore_patterns {
        builder.add_ignore(pattern);
    }

    builder
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if entry.file_type()?.is_file() { Some(entry.into_path()) } else { None }
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
                    extensions.contains(ext)
                }
            })
        })
        .collect()
}
