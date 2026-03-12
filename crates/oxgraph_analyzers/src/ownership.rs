use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use rustc_hash::{FxHashMap, FxHashSet};

use oxgraph_config::{AnalysisSeverity, OwnershipConfig};
use oxgraph_fs::is_docs_path;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Evidence, Finding, FindingCategory};

use crate::{make_finding, severity};

/// Find files without an inferred owner and cross-owner dependency edges.
#[allow(clippy::too_many_lines)]
pub fn analyze(
    build: &GraphBuildResult,
    ownership: Option<&OwnershipConfig>,
    level: AnalysisSeverity,
) -> Vec<Finding> {
    let Some(config) = ownership else {
        return Vec::new();
    };
    let Some(finding_severity) = severity(level) else {
        return Vec::new();
    };

    let team_matchers = compile_team_matchers(config);
    let owners = build
        .files
        .iter()
        .map(|file| {
            (
                file.file.path.clone(),
                owner_for_file(
                    build,
                    &team_matchers,
                    &file.file.relative_path.to_string_lossy(),
                ),
            )
        })
        .collect::<FxHashMap<_, _>>();

    let mut findings = Vec::new();
    for extracted_file in &build.files {
        if should_skip_file(extracted_file.file.role, &extracted_file.file.relative_path) {
            continue;
        }

        let relative_path = extracted_file.file.relative_path.to_string_lossy();
        let owner = owners.get(&extracted_file.file.path).cloned().flatten();
        if owner.is_none() {
            findings.push(make_finding(
                "ownership-unowned",
                finding_severity,
                FindingCategory::OwnershipViolation,
                relative_path.as_ref(),
                extracted_file.file.workspace.clone(),
                extracted_file.file.package.clone(),
                format!("File `{relative_path}` has no matching CODEOWNERS or ownership team."),
                vec![Evidence {
                    kind: "ownership".to_string(),
                    file: Some(relative_path.to_string()),
                    line: None,
                    description: "No ownership rule matched this tracked file.".to_string(),
                }],
                Some(
                    "Add a CODEOWNERS rule or configure an owning team for this path.".to_string(),
                ),
                None,
            ));
        }
    }

    let mut seen_cross_edges = FxHashSet::default();
    for extracted_file in &build.files {
        if should_skip_file(extracted_file.file.role, &extracted_file.file.relative_path) {
            continue;
        }

        let Some(source_owner) = owners.get(&extracted_file.file.path).cloned().flatten() else {
            continue;
        };

        for edge in extracted_file
            .resolved_imports
            .iter()
            .chain(&extracted_file.resolved_reexports)
        {
            let Some(target_file) = &edge.to_file else {
                continue;
            };
            let Some(target_owner) = owners.get(target_file).cloned().flatten() else {
                continue;
            };
            if source_owner == target_owner {
                continue;
            }

            let Some(target) = build.find_file(&target_file.to_string_lossy()) else {
                continue;
            };

            let key = (
                extracted_file.file.relative_path.clone(),
                target.file.relative_path.clone(),
                source_owner.clone(),
                target_owner.clone(),
            );
            if !seen_cross_edges.insert(key) {
                continue;
            }

            findings.push(make_finding(
                "ownership-cross-owner",
                finding_severity,
                FindingCategory::OwnershipViolation,
                format!(
                    "{} -> {}",
                    extracted_file.file.relative_path.to_string_lossy(),
                    target.file.relative_path.to_string_lossy()
                ),
                extracted_file.file.workspace.clone(),
                extracted_file.file.package.clone(),
                format!(
                    "Cross-owner dependency from `{}` ({source_owner}) to `{}` ({target_owner}).",
                    extracted_file.file.relative_path.to_string_lossy(),
                    target.file.relative_path.to_string_lossy()
                ),
                vec![Evidence {
                    kind: "ownership".to_string(),
                    file: Some(extracted_file.file.relative_path.to_string_lossy().to_string()),
                    line: edge.line.map(|line| line as usize),
                    description: format!(
                        "Dependency crosses ownership boundary: {source_owner} -> {target_owner}."
                    ),
                }],
                Some(
                    "Review whether this dependency direction is intentional or split/shared ownership is needed."
                        .to_string(),
                ),
                None,
            ));
        }
    }

    findings
}

fn compile_team_matchers(
    config: &OwnershipConfig,
) -> Vec<(String, GlobSet)> {
    let mut matchers = config
        .teams
        .as_ref()
        .map(|teams| {
            teams
                .iter()
                .filter_map(|(team, config)| compile_globset(&config.paths).map(|matcher| (team.clone(), matcher)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    matchers.sort_by(|left, right| left.0.cmp(&right.0));
    matchers
}

fn owner_for_file(
    build: &GraphBuildResult,
    team_matchers: &[(String, GlobSet)],
    relative_path: &str,
) -> Option<String> {
    if let Some(codeowners) = &build.discovery.codeowners
        && let Some(owners) = match_codeowners(codeowners, relative_path)
    {
        return Some(owners.join(" "));
    }

    team_matchers
        .iter()
        .find(|(_, matcher)| matcher.is_match(relative_path))
        .map(|(team, _)| team.clone())
}

fn should_skip_file(role: oxgraph_fs::FileRole, relative_path: &Path) -> bool {
    role.excluded_from_dead_code_by_default() || is_docs_path(relative_path)
}

fn match_codeowners<'a>(
    codeowners: &'a oxgraph_discovery::Codeowners,
    relative_path: &str,
) -> Option<&'a [String]> {
    let mut matched = None;
    for rule in &codeowners.rules {
        if codeowners_pattern_matches(&rule.pattern, relative_path) {
            matched = Some(rule.owners.as_slice());
        }
    }
    matched
}

fn codeowners_pattern_matches(pattern: &str, relative_path: &str) -> bool {
    let normalized = pattern.trim_start_matches('/');
    let glob = if normalized.ends_with('/') {
        format!("{normalized}**")
    } else if normalized.contains('*') {
        normalized.to_string()
    } else {
        format!("{normalized}**")
    };

    Glob::new(&glob)
        .ok()
        .is_some_and(|compiled| compiled.compile_matcher().is_match(relative_path))
}

fn compile_globset(patterns: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
            has_patterns = true;
        }
    }

    if !has_patterns {
        return None;
    }

    builder.build().ok()
}
