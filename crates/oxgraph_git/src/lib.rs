use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedScope {
    pub reference: String,
    pub merge_base: String,
    pub added: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub renamed: Vec<RenamedPath>,
    pub deleted: Vec<PathBuf>,
    pub recoverable_deleted: Vec<PathBuf>,
    pub unrecoverable_deleted: Vec<PathBuf>,
}

impl ChangedScope {
    pub fn changed_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        paths.extend(self.added.iter().cloned());
        paths.extend(self.modified.iter().cloned());
        paths.extend(self.deleted.iter().cloned());
        paths.extend(self.renamed.iter().map(|rename| rename.from.clone()));
        paths.extend(self.renamed.iter().map(|rename| rename.to.clone()));
        paths.sort();
        paths.dedup();
        paths
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamedPath {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum GitError {
    #[error("`{0}` is not inside a git repository")]
    NotRepository(String),
    #[error("failed to run git command: {0}")]
    CommandFailed(String),
    #[error("failed to parse git diff output: {0}")]
    ParseError(String),
}

pub fn collect_changed_scope(
    project_root: &Path,
    reference: &str,
    scan_roots: &[PathBuf],
) -> miette::Result<ChangedScope> {
    if !is_git_repository(project_root)? {
        return Err(GitError::NotRepository(project_root.display().to_string()).into());
    }

    let merge_base = git_stdout(project_root, &["merge-base", reference, "HEAD"])?;
    let merge_base = merge_base.trim().to_string();
    if merge_base.is_empty() {
        return Err(GitError::CommandFailed("empty merge-base result".to_string()).into());
    }

    let mut args = vec![
        "diff".to_string(),
        "--name-status".to_string(),
        "--find-renames".to_string(),
        merge_base.clone(),
        "HEAD".to_string(),
    ];
    if !scan_roots.is_empty() {
        args.push("--".to_string());
        for root in scan_roots {
            let relative = root.strip_prefix(project_root).unwrap_or(root);
            let relative = relative.to_string_lossy();
            if !relative.is_empty() && relative != "." {
                args.push(relative.to_string());
            }
        }
    }

    let output = git_stdout(
        project_root,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;

    let mut scope = ChangedScope {
        reference: reference.to_string(),
        merge_base,
        added: Vec::new(),
        modified: Vec::new(),
        renamed: Vec::new(),
        deleted: Vec::new(),
        recoverable_deleted: Vec::new(),
        unrecoverable_deleted: Vec::new(),
    };

    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split('\t');
        let status = parts
            .next()
            .ok_or_else(|| GitError::ParseError(format!("missing status in `{line}`")))?;

        if status.starts_with('R') {
            let from = parts
                .next()
                .ok_or_else(|| GitError::ParseError(format!("missing rename source in `{line}`")))?;
            let to = parts.next().ok_or_else(|| {
                GitError::ParseError(format!("missing rename destination in `{line}`"))
            })?;
            scope.renamed.push(RenamedPath {
                from: PathBuf::from(from),
                to: PathBuf::from(to),
            });
            continue;
        }

        let path = parts
            .next()
            .ok_or_else(|| GitError::ParseError(format!("missing path in `{line}`")))?;

        match status.chars().next() {
            Some('A') => scope.added.push(PathBuf::from(path)),
            Some('M' | 'T' | 'C') => scope.modified.push(PathBuf::from(path)),
            Some('D') => scope.deleted.push(PathBuf::from(path)),
            Some(other) => {
                return Err(GitError::ParseError(format!(
                    "unsupported git diff status `{other}` in `{line}`"
                ))
                .into());
            }
            None => return Err(GitError::ParseError(format!("empty status in `{line}`")).into()),
        }
    }

    scope.added.sort();
    scope.modified.sort();
    scope.deleted.sort();
    scope.renamed.sort_by(|left, right| left.from.cmp(&right.from).then(left.to.cmp(&right.to)));

    Ok(scope)
}

fn is_git_repository(project_root: &Path) -> miette::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|err| GitError::CommandFailed(err.to_string()))?;
    Ok(output.status.success())
}

fn git_stdout(project_root: &Path, args: &[&str]) -> miette::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()
        .map_err(|err| GitError::CommandFailed(err.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("git {} failed with status {}", args.join(" "), output.status)
        } else {
            stderr
        };
        return Err(GitError::CommandFailed(message).into());
    }

    String::from_utf8(output.stdout)
        .map_err(|err| GitError::CommandFailed(err.to_string()).into())
}
