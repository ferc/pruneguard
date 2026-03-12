use globset::{Glob, GlobSet, GlobSetBuilder};

use oxgraph_config::{AnalysisSeverity, OwnershipConfig};
use oxgraph_fs::FileKind;
use oxgraph_graph::GraphBuildResult;
use oxgraph_report::{Evidence, Finding, FindingCategory};

use crate::{make_finding, severity};

/// Find files without an inferred owner.
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

    let mut findings = Vec::new();
    let team_matchers = config
        .teams
        .as_ref()
        .map(|teams| {
            teams
                .iter()
                .filter_map(|(team, config)| compile_globset(&config.paths).map(|matcher| (team.clone(), matcher)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for extracted_file in &build.files {
        if matches!(extracted_file.file.kind, FileKind::Generated | FileKind::BuildOutput) {
            continue;
        }

        let relative_path = extracted_file.file.relative_path.to_string_lossy();
        let owned_by_codeowners = build
            .discovery
            .codeowners
            .as_ref()
            .and_then(|codeowners| match_codeowners(codeowners, &relative_path));
        let owned_by_team = team_matchers
            .iter()
            .find(|(_, matcher)| matcher.is_match(relative_path.as_ref()))
            .map(|(team, _)| team.as_str());

        if owned_by_codeowners.is_some() || owned_by_team.is_some() {
            continue;
        }

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
            Some("Add a CODEOWNERS rule or configure an owning team for this path.".to_string()),
            None,
        ));
    }

    findings
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
