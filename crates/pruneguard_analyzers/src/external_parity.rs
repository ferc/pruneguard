//! External parity harness: fixture-derived parity scoring.
//!
//! Replaces the hand-authored parity matrix with a corpus of test fixtures,
//! each containing source files, expected findings, and expected reachability.
//! The harness discovers cases under a corpus root directory, evaluates them,
//! and produces aggregate parity scores by family and reference tool.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Metadata for a parity test case.
#[derive(Debug, Clone, Deserialize)]
pub struct ParityCaseMeta {
    pub family: String,
    pub name: String,
    pub reference_tool: String,
    pub description: String,
}

/// Expected outcomes for a parity test case.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ParityCaseExpected {
    #[serde(default)]
    pub reachable_files: Vec<String>,
    #[serde(default)]
    pub unreachable_files: Vec<String>,
    #[serde(default)]
    pub expected_findings: Vec<String>,
    #[serde(default)]
    pub expected_no_findings: Vec<String>,
}

/// Result of evaluating a single parity case.
#[derive(Debug, Clone, Serialize)]
pub struct ParityCaseResult {
    pub family: String,
    pub name: String,
    pub reference_tool: String,
    pub passed: bool,
    pub total_checks: usize,
    pub passed_checks: usize,
    pub failures: Vec<String>,
}

/// Aggregate parity score from the external corpus.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalParityScore {
    pub total_cases: usize,
    pub passed_cases: usize,
    pub total_checks: usize,
    pub passed_checks: usize,
    pub overall_pct: f64,
    pub by_family: Vec<FamilyParityScore>,
    pub by_reference_tool: Vec<ToolParityScore>,
    pub case_results: Vec<ParityCaseResult>,
}

/// Per-family parity score.
#[derive(Debug, Clone, Serialize)]
pub struct FamilyParityScore {
    pub family: String,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub pct: f64,
}

/// Per-reference-tool parity score.
#[derive(Debug, Clone, Serialize)]
pub struct ToolParityScore {
    pub tool: String,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub pct: f64,
}

/// Discover all parity cases under the given corpus root.
///
/// The corpus root is expected to contain family subdirectories, each
/// containing case subdirectories. Each case must have a `meta.json` file;
/// an `expected.json` file is optional (defaults to empty expectations).
///
/// Returns a sorted list of (meta, expected, case_dir) tuples.
pub fn discover_parity_cases(
    corpus_root: &Path,
) -> Vec<(ParityCaseMeta, ParityCaseExpected, PathBuf)> {
    let mut cases = Vec::new();
    if !corpus_root.exists() {
        return cases;
    }

    // Walk family directories.
    let Ok(families) = std::fs::read_dir(corpus_root) else {
        return cases;
    };

    for family_entry in families.flatten() {
        if !family_entry.file_type().map_or(false, |ft| ft.is_dir()) {
            continue;
        }
        let Ok(cases_in_family) = std::fs::read_dir(family_entry.path()) else {
            continue;
        };
        for case_entry in cases_in_family.flatten() {
            if !case_entry.file_type().map_or(false, |ft| ft.is_dir()) {
                continue;
            }
            let case_dir = case_entry.path();
            let meta_path = case_dir.join("meta.json");
            let expected_path = case_dir.join("expected.json");

            let Ok(meta_content) = std::fs::read_to_string(&meta_path) else {
                continue;
            };
            let Ok(meta): Result<ParityCaseMeta, _> = serde_json::from_str(&meta_content) else {
                continue;
            };

            let expected = if expected_path.exists() {
                std::fs::read_to_string(&expected_path)
                    .ok()
                    .and_then(|c| serde_json::from_str(&c).ok())
                    .unwrap_or_default()
            } else {
                ParityCaseExpected::default()
            };

            cases.push((meta, expected, case_dir));
        }
    }

    cases.sort_by(|a, b| a.0.family.cmp(&b.0.family).then(a.0.name.cmp(&b.0.name)));
    cases
}

/// Compute the aggregate external parity score from a slice of case results.
pub fn compute_external_parity_score(results: &[ParityCaseResult]) -> ExternalParityScore {
    let total_cases = results.len();
    let passed_cases = results.iter().filter(|r| r.passed).count();
    let total_checks: usize = results.iter().map(|r| r.total_checks).sum();
    let passed_checks: usize = results.iter().map(|r| r.passed_checks).sum();
    let overall_pct =
        if total_checks == 0 { 0.0 } else { (passed_checks as f64 / total_checks as f64) * 100.0 };

    // Group by family.
    let mut family_map: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in results {
        let entry = family_map.entry(r.family.clone()).or_default();
        entry.0 += 1;
        if r.passed {
            entry.1 += 1;
        }
    }
    let by_family = family_map
        .into_iter()
        .map(|(family, (total, passed))| FamilyParityScore {
            family,
            total_cases: total,
            passed_cases: passed,
            pct: if total == 0 { 0.0 } else { (passed as f64 / total as f64) * 100.0 },
        })
        .collect();

    // Group by reference tool.
    let mut tool_map: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in results {
        let entry = tool_map.entry(r.reference_tool.clone()).or_default();
        entry.0 += 1;
        if r.passed {
            entry.1 += 1;
        }
    }
    let by_reference_tool = tool_map
        .into_iter()
        .map(|(tool, (total, passed))| ToolParityScore {
            tool,
            total_cases: total,
            passed_cases: passed,
            pct: if total == 0 { 0.0 } else { (passed as f64 / total as f64) * 100.0 },
        })
        .collect();

    ExternalParityScore {
        total_cases,
        passed_cases,
        total_checks,
        passed_checks,
        overall_pct,
        by_family,
        by_reference_tool,
        case_results: results.to_vec(),
    }
}

/// Format the external parity score as a human-readable report.
pub fn format_external_parity_report(score: &ExternalParityScore) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "External Parity Score: {:.1}% ({}/{} checks passed)",
        score.overall_pct, score.passed_checks, score.total_checks
    );
    let _ = writeln!(out, "Cases: {}/{} fully passing", score.passed_cases, score.total_cases);
    let _ = writeln!(out);

    let _ = writeln!(out, "By family:");
    for f in &score.by_family {
        let _ = writeln!(
            out,
            "  {:<30} {}/{} ({:.1}%)",
            f.family, f.passed_cases, f.total_cases, f.pct
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "By reference tool:");
    for t in &score.by_reference_tool {
        let _ =
            writeln!(out, "  {:<30} {}/{} ({:.1}%)", t.tool, t.passed_cases, t.total_cases, t.pct);
    }

    let failed: Vec<_> = score.case_results.iter().filter(|r| !r.passed).collect();
    if !failed.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Failed cases:");
        for r in &failed {
            let _ = writeln!(
                out,
                "  {}/{}: {}/{} checks",
                r.family, r.name, r.passed_checks, r.total_checks
            );
            for failure in &r.failures {
                let _ = writeln!(out, "    - {failure}");
            }
        }
    }

    out
}
