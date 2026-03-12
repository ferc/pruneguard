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
            for value in map.values() {
                collect_export_paths(value, output);
            }
        }
        serde_json::Value::Bool(_)
        | serde_json::Value::Null
        | serde_json::Value::Number(_) => {}
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
