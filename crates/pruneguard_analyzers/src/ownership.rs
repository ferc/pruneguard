use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rustc_hash::{FxHashMap, FxHashSet};

use pruneguard_config::{AnalysisSeverity, OwnershipConfig};
use pruneguard_fs::is_docs_path;
use pruneguard_graph::GraphBuildResult;
use pruneguard_report::{Evidence, Finding, FindingCategory, FindingConfidence, FindingSeverity};

use crate::{make_finding, severity};

/// Find files without an inferred owner and cross-owner dependency edges.
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
        .map(|file| (file.file.path.clone(), owner_for_file(build, &team_matchers, file)))
        .collect::<FxHashMap<_, _>>();

    let mut findings = find_unowned_files(build, &owners, finding_severity);

    let (cross_owner_findings, hotspots) = find_cross_owner_edges(build, &owners, finding_severity);
    findings.extend(cross_owner_findings);

    findings.extend(find_ownership_hotspots(hotspots));

    findings
}

type HotspotMap = FxHashMap<String, (FxHashSet<String>, Option<String>, Option<String>, usize)>;

fn find_unowned_files(
    build: &GraphBuildResult,
    owners: &FxHashMap<PathBuf, Option<String>>,
    finding_severity: FindingSeverity,
) -> Vec<Finding> {
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
                FindingConfidence::High,
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
    findings
}

fn find_cross_owner_edges(
    build: &GraphBuildResult,
    owners: &FxHashMap<PathBuf, Option<String>>,
    finding_severity: FindingSeverity,
) -> (Vec<Finding>, HotspotMap) {
    let mut findings = Vec::new();
    let mut seen_cross_edges = FxHashSet::default();
    let mut hotspots = HotspotMap::default();

    for extracted_file in &build.files {
        if should_skip_file(extracted_file.file.role, &extracted_file.file.relative_path) {
            continue;
        }

        let Some(source_owner) = owners.get(&extracted_file.file.path).cloned().flatten() else {
            continue;
        };

        for edge in extracted_file.resolved_imports.iter().chain(&extracted_file.resolved_reexports)
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

            let entry = hotspots
                .entry(target.file.relative_path.to_string_lossy().to_string())
                .or_insert_with(|| {
                    (
                        FxHashSet::default(),
                        target.file.workspace.clone(),
                        target.file.package.clone(),
                        0,
                    )
                });
            entry.0.insert(source_owner.clone());
            entry.3 += 1;

            findings.push(make_finding(
                "ownership-cross-owner",
                finding_severity,
                FindingCategory::OwnershipViolation,
                FindingConfidence::High,
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

    (findings, hotspots)
}

fn find_ownership_hotspots(mut hotspots: HotspotMap) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut hotspot_paths = hotspots.keys().cloned().collect::<Vec<_>>();
    hotspot_paths.sort();
    for relative_path in hotspot_paths {
        let Some((owners_seen, workspace, package, edge_count)) = hotspots.remove(&relative_path)
        else {
            continue;
        };
        if owners_seen.len() < 2 {
            continue;
        }

        findings.push(make_finding(
            "ownership-hotspot",
            FindingSeverity::Info,
            FindingCategory::OwnershipViolation,
            FindingConfidence::High,
            &relative_path,
            workspace,
            package,
            format!(
                "File `{relative_path}` is a shared ownership hotspot with {edge_count} cross-owner incoming edges."
            ),
            vec![Evidence {
                kind: "ownership".to_string(),
                file: Some(relative_path.clone()),
                line: None,
                description: format!(
                    "Cross-owner callers: {}.",
                    owners_seen.into_iter().collect::<Vec<_>>().join(", ")
                ),
            }],
            Some("Consider splitting the file, narrowing dependencies, or assigning shared ownership explicitly.".to_string()),
            None,
        ));
    }
    findings
}

struct TeamMatcher {
    team: String,
    path_matcher: Option<GlobSet>,
    packages: FxHashSet<String>,
}

fn compile_team_matchers(config: &OwnershipConfig) -> Vec<TeamMatcher> {
    let mut matchers = config
        .teams
        .as_ref()
        .map(|teams| {
            teams
                .iter()
                .map(|(team, config)| TeamMatcher {
                    team: team.clone(),
                    path_matcher: compile_globset(&config.paths),
                    packages: config.packages.iter().cloned().collect(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    matchers.sort_by(|left, right| left.team.cmp(&right.team));
    matchers
}

fn owner_for_file(
    build: &GraphBuildResult,
    team_matchers: &[TeamMatcher],
    file: &pruneguard_extract::ExtractedFile,
) -> Option<String> {
    let relative_path = file.file.relative_path.to_string_lossy();
    if let Some(team) = team_matchers.iter().find(|matcher| {
        matcher.path_matcher.as_ref().is_some_and(|glob| glob.is_match(relative_path.as_ref()))
            || file.file.package.as_ref().is_some_and(|package| matcher.packages.contains(package))
    }) {
        return Some(team.team.clone());
    }

    if let Some(codeowners) = &build.discovery.codeowners
        && let Some(owners) = match_codeowners(codeowners, relative_path.as_ref())
    {
        return Some(owners.join(" "));
    }

    None
}

fn should_skip_file(role: pruneguard_fs::FileRole, relative_path: &Path) -> bool {
    role.excluded_from_dead_code_by_default() || is_docs_path(relative_path)
}

fn match_codeowners<'a>(
    codeowners: &'a pruneguard_discovery::Codeowners,
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

    Glob::new(&glob).ok().is_some_and(|compiled| compiled.compile_matcher().is_match(relative_path))
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
