use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// A parsed `package.json` manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub private: Option<bool>,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub types: Option<String>,
    #[serde(rename = "type", default)]
    pub module_type: Option<String>,
    #[serde(default)]
    pub bin: Option<BinField>,
    #[serde(default)]
    pub exports: Option<serde_json::Value>,
    #[serde(default)]
    pub workspaces: Option<WorkspacesField>,
    #[serde(default)]
    pub scripts: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub dependencies: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub dev_dependencies: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub peer_dependencies: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub optional_dependencies: Option<FxHashMap<String, String>>,
}

/// The `bin` field can be a string or a map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BinField {
    Single(String),
    Map(FxHashMap<String, String>),
}

/// The `workspaces` field can be an array or an object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkspacesField {
    Array(Vec<String>),
    Object { packages: Option<Vec<String>>, nohoist: Option<Vec<String>> },
}

impl WorkspacesField {
    /// Get the workspace package globs.
    pub fn patterns(&self) -> &[String] {
        match self {
            Self::Array(patterns) => patterns,
            Self::Object { packages, .. } => packages.as_deref().unwrap_or_default(),
        }
    }
}

impl PackageManifest {
    /// Load a package.json from the given path.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)
            .map_err(|source| ManifestError::ReadError { path: path.to_path_buf(), source })?;
        serde_json::from_str(&content)
            .map_err(|source| ManifestError::ParseError { path: path.to_path_buf(), source })
    }

    /// Get all dependency names (production + peer + optional).
    pub fn production_dependencies(&self) -> impl Iterator<Item = &str> {
        self.dependencies
            .iter()
            .flat_map(|deps| deps.keys())
            .chain(self.peer_dependencies.iter().flat_map(|deps| deps.keys()))
            .chain(self.optional_dependencies.iter().flat_map(|deps| deps.keys()))
            .map(String::as_str)
    }

    /// Get all dev dependency names.
    pub fn dev_dependencies_names(&self) -> impl Iterator<Item = &str> {
        self.dev_dependencies.iter().flat_map(|deps| deps.keys()).map(String::as_str)
    }

    /// Get all entrypoint files from main, module, types, bin, and exports.
    pub fn entrypoint_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        if let Some(main) = &self.main {
            files.push(main.clone());
        }
        if let Some(module) = &self.module {
            files.push(module.clone());
        }
        if let Some(types) = &self.types {
            files.push(types.clone());
        }
        if let Some(bin) = &self.bin {
            match bin {
                BinField::Single(path) => files.push(path.clone()),
                BinField::Map(map) => files.extend(map.values().cloned()),
            }
        }
        if let Some(exports) = &self.exports {
            collect_export_paths(exports, &mut files);
        }
        files.sort();
        files.dedup();
        files
    }

    /// Return all subpath patterns defined in the exports map.
    /// E.g. for `"exports": { ".": ..., "./utils": ... }` this returns `[".", "./utils"]`.
    pub fn exports_subpaths(&self) -> Vec<String> {
        let Some(exports) = &self.exports else {
            return Vec::new();
        };
        let mut subpaths = Vec::new();
        collect_exports_subpaths(exports, &mut subpaths);
        subpaths.sort();
        subpaths.dedup();
        subpaths
    }

    /// Check whether the exports map defines a `types` condition for any subpath.
    pub fn exports_has_types_condition(&self) -> bool {
        self.exports.as_ref().is_some_and(|exports| exports_contains_condition(exports, "types"))
    }

    /// Check whether the exports map defines a specific subpath (exact or wildcard).
    pub fn exports_defines_subpath(&self, subpath: &str) -> bool {
        let Some(exports) = &self.exports else {
            return false;
        };
        match exports {
            serde_json::Value::Object(map) => {
                let is_subpath_map = map.keys().any(|k| k.starts_with('.'));
                if !is_subpath_map {
                    // Condition-only map, applies to "."
                    return subpath == ".";
                }
                // Exact match.
                if map.contains_key(subpath) {
                    return true;
                }
                // Wildcard/pattern match.
                map.keys().any(|pattern| {
                    if let Some(prefix) = pattern.strip_suffix('*') {
                        subpath.starts_with(prefix)
                    } else {
                        false
                    }
                })
            }
            serde_json::Value::String(_) => subpath == ".",
            _ => false,
        }
    }
}

fn collect_export_paths(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(path) => {
            if looks_like_entrypoint(path) {
                output.push(path.clone());
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_export_paths(value, output);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                // Skip wildcard patterns in subpath exports — they are glob patterns,
                // not concrete file paths.
                if key.contains('*') {
                    continue;
                }
                collect_export_paths(value, output);
            }
        }
        serde_json::Value::Bool(_) | serde_json::Value::Null | serde_json::Value::Number(_) => {}
    }
}

/// Collect subpath patterns from an exports map.
fn collect_exports_subpaths(value: &serde_json::Value, output: &mut Vec<String>) {
    if let serde_json::Value::Object(map) = value {
        let is_subpath_map = map.keys().any(|k| k.starts_with('.'));
        if is_subpath_map {
            for key in map.keys() {
                if key.starts_with('.') {
                    output.push(key.clone());
                }
            }
        }
        // If this is a condition map (keys are "import", "require", etc.),
        // don't recurse — conditions are not subpaths.
    }
}

/// Check if any branch of the exports value uses a particular condition name.
fn exports_contains_condition(value: &serde_json::Value, condition: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if map.contains_key(condition) {
                return true;
            }
            map.values().any(|v| exports_contains_condition(v, condition))
        }
        serde_json::Value::Array(arr) => {
            arr.iter().any(|v| exports_contains_condition(v, condition))
        }
        _ => false,
    }
}

fn looks_like_entrypoint(value: &str) -> bool {
    if value.to_ascii_lowercase().ends_with(".d.ts") {
        return true;
    }

    Path::new(value).extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "tsx" | "jsx"
        )
    })
}

/// Errors that can occur when loading a manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read {path}")]
    ReadError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}")]
    ParseError {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}
