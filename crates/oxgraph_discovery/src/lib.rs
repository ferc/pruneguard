use std::path::{Path, PathBuf};

use oxgraph_config::{OxgraphConfig, PackageManager};
use oxgraph_fs::{FileCollectionOptions, FileRecord, FileRole, collect_file_records};
use oxgraph_manifest::PackageManifest;
use rustc_hash::FxHashMap;

/// A discovered workspace in the monorepo.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Workspace name (from package.json or directory name).
    pub name: String,
    /// Root directory of this workspace.
    pub root: PathBuf,
    /// Parsed package.json manifest.
    pub manifest: PackageManifest,
}

/// Discovery result containing all workspaces and their packages.
#[derive(Debug, Default)]
pub struct DiscoveryResult {
    /// The project root directory.
    pub project_root: PathBuf,
    /// All discovered workspaces, keyed by workspace name.
    pub workspaces: FxHashMap<String, Workspace>,
    /// Workspace dependency graph (workspace name -> dependencies).
    pub workspace_deps: FxHashMap<String, Vec<String>>,
    /// CODEOWNERS data, if found.
    pub codeowners: Option<Codeowners>,
}

impl DiscoveryResult {
    /// Return workspace roots keyed by workspace name.
    pub fn workspace_roots(&self) -> FxHashMap<String, PathBuf> {
        self.workspaces
            .iter()
            .map(|(name, workspace)| (name.clone(), workspace.root.clone()))
            .collect()
    }

    /// Return package names keyed by workspace name.
    pub fn package_names(&self) -> FxHashMap<String, String> {
        self.workspaces
            .iter()
            .map(|(name, workspace)| {
                (
                    name.clone(),
                    workspace
                        .manifest
                        .name
                        .clone()
                        .unwrap_or_else(|| name.clone()),
                )
            })
            .collect()
    }

    /// Collect tracked files under the project root.
    pub fn collect_files(&self, config: &OxgraphConfig) -> Vec<FileRecord> {
        let options = FileCollectionOptions {
            ignore_patterns: config.ignore_patterns.clone(),
            workspace_roots: self.workspace_roots(),
            package_names: self.package_names(),
            extra_classifications: vec![
                ("**/*.stories.*".to_string(), FileRole::Story),
                ("**/*.story.*".to_string(), FileRole::Story),
                ("**/*.test.*".to_string(), FileRole::Test),
                ("**/*.spec.*".to_string(), FileRole::Test),
                ("**/__tests__/**".to_string(), FileRole::Test),
                ("fixtures/**".to_string(), FileRole::Fixture),
                ("**/fixtures/**".to_string(), FileRole::Fixture),
                ("examples/**".to_string(), FileRole::Example),
                ("**/examples/**".to_string(), FileRole::Example),
                ("templates/**".to_string(), FileRole::Template),
                ("**/templates/**".to_string(), FileRole::Template),
                ("benchmarks/**".to_string(), FileRole::Benchmark),
                ("**/benchmarks/**".to_string(), FileRole::Benchmark),
            ],
        };

        collect_file_records(&self.project_root, &options)
    }
}

/// Parsed CODEOWNERS file.
#[derive(Debug, Clone, Default)]
pub struct Codeowners {
    pub rules: Vec<CodeownersRule>,
}

/// A single CODEOWNERS rule.
#[derive(Debug, Clone)]
pub struct CodeownersRule {
    pub pattern: String,
    pub owners: Vec<String>,
}

/// Discover all workspaces and packages in the project.
pub fn discover(cwd: &Path, config: &OxgraphConfig) -> miette::Result<DiscoveryResult> {
    let project_root = find_project_root(cwd);
    let mut result = DiscoveryResult { project_root: project_root.clone(), ..Default::default() };

    // Load root manifest
    let root_manifest_path = project_root.join("package.json");
    if root_manifest_path.exists() {
        let root_manifest =
            PackageManifest::load(&root_manifest_path).map_err(|e| miette::miette!("{e}"))?;
        let root_name = root_manifest.name.clone().unwrap_or_else(|| {
            project_root
                .file_name()
                .map_or_else(|| "root".to_string(), |name| name.to_string_lossy().to_string())
        });
        result.workspaces.insert(
            root_name.clone(),
            Workspace {
                name: root_name,
                root: project_root.clone(),
                manifest: root_manifest.clone(),
            },
        );

        // Detect workspace globs from config or package.json
        let workspace_globs = if let Some(ws_config) = &config.workspaces {
            if ws_config.roots.is_empty() {
                extract_workspace_globs(&root_manifest, &project_root, config)
            } else {
                ws_config.roots.clone()
            }
        } else {
            extract_workspace_globs(&root_manifest, &project_root, config)
        };

        // Discover workspace directories
        for glob_pattern in &workspace_globs {
            let pattern = project_root.join(glob_pattern).display().to_string();
            if let Ok(paths) = glob::glob(&pattern) {
                for entry in paths.flatten() {
                    let manifest_path = entry.join("package.json");
                    if manifest_path.exists()
                        && let Ok(manifest) = PackageManifest::load(&manifest_path)
                    {
                        let name = manifest.name.clone().unwrap_or_else(|| {
                            entry
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default()
                        });
                        result.workspaces.entry(name.clone()).or_insert_with(|| Workspace {
                            name,
                            root: entry,
                            manifest,
                        });
                    }
                }
            }
        }
    }

    // Load CODEOWNERS if ownership is configured
    if config.ownership.as_ref().is_some_and(|o| o.import_codeowners) {
        result.codeowners = load_codeowners(&project_root);
    }

    // Build workspace dependency graph
    for (name, workspace) in &result.workspaces {
        let mut deps = Vec::new();
        for dep_name in workspace.manifest.production_dependencies() {
            if result.workspaces.contains_key(dep_name) {
                deps.push(dep_name.to_string());
            }
        }
        for dep_name in workspace.manifest.dev_dependencies_names() {
            if result.workspaces.contains_key(dep_name) {
                deps.push(dep_name.to_string());
            }
        }
        result.workspace_deps.insert(name.clone(), deps);
    }

    Ok(result)
}

/// Find the project root by searching for a root package.json or git root.
fn find_project_root(cwd: &Path) -> PathBuf {
    if cwd.join("package.json").exists() {
        return cwd.to_path_buf();
    }

    let mut current = cwd;
    let mut package_fallback = None;
    loop {
        if current.join("pnpm-workspace.yaml").exists()
            || current.join("lerna.json").exists()
        {
            return current.to_path_buf();
        }
        let package_json = current.join("package.json");
        if package_json.exists() {
            package_fallback.get_or_insert_with(|| current.to_path_buf());
            if package_json_has_workspaces(&package_json) {
                return current.to_path_buf();
            }
        }
        if current.join(".git").exists() {
            return package_fallback.unwrap_or_else(|| current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return package_fallback.unwrap_or_else(|| cwd.to_path_buf()),
        }
    }
}

fn package_json_has_workspaces(package_json: &Path) -> bool {
    std::fs::read_to_string(package_json)
        .ok()
        .is_some_and(|content| content.contains("\"workspaces\""))
}

/// Extract workspace globs from package manager config files.
fn extract_workspace_globs(
    manifest: &PackageManifest,
    project_root: &Path,
    config: &OxgraphConfig,
) -> Vec<String> {
    let package_manager =
        config.workspaces.as_ref().map_or(PackageManager::Auto, |ws| ws.package_manager);

    match package_manager {
        PackageManager::Pnpm | PackageManager::Auto => {
            // Try pnpm-workspace.yaml first
            let pnpm_ws_path = project_root.join("pnpm-workspace.yaml");
            if pnpm_ws_path.exists()
                && let Ok(content) = std::fs::read_to_string(&pnpm_ws_path)
                && let Some(globs) = parse_pnpm_workspace(&content)
            {
                return globs;
            }
            // Fall back to package.json workspaces
            if let Some(workspaces) = &manifest.workspaces {
                return workspaces.patterns().to_vec();
            }
            Vec::new()
        }
        PackageManager::Npm | PackageManager::Yarn | PackageManager::Bun => {
            if let Some(workspaces) = &manifest.workspaces {
                workspaces.patterns().to_vec()
            } else {
                Vec::new()
            }
        }
    }
}

/// Parse a `pnpm-workspace.yaml` file for package globs.
fn parse_pnpm_workspace(content: &str) -> Option<Vec<String>> {
    // Simple YAML parsing for the packages field
    let mut in_packages = false;
    let mut globs = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "packages:" {
            in_packages = true;
            continue;
        }
        if in_packages {
            if let Some(glob) = trimmed.strip_prefix("- ") {
                let glob = glob.trim().trim_matches('"').trim_matches('\'');
                globs.push(glob.to_string());
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                break;
            }
        }
    }

    if globs.is_empty() { None } else { Some(globs) }
}

/// Load CODEOWNERS file from standard locations.
fn load_codeowners(project_root: &Path) -> Option<Codeowners> {
    let candidates = [
        project_root.join("CODEOWNERS"),
        project_root.join(".github/CODEOWNERS"),
        project_root.join("docs/CODEOWNERS"),
    ];

    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            return Some(parse_codeowners(&content));
        }
    }
    None
}

/// Parse a CODEOWNERS file.
fn parse_codeowners(content: &str) -> Codeowners {
    let rules = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(CodeownersRule {
                    pattern: parts[0].to_string(),
                    owners: parts[1..].iter().map(|s| (*s).to_string()).collect(),
                })
            } else {
                None
            }
        })
        .collect();

    Codeowners { rules }
}
