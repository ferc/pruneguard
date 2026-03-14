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

// ---------------------------------------------------------------------------
// Weighted replacement score
// ---------------------------------------------------------------------------

/// Weights for the replacement score components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacementWeights {
    /// Weight for the parity corpus pass rate (default 0.50).
    pub parity_corpus: f64,
    /// Weight for canary repo pass rate (default 0.30).
    pub canary_repos: f64,
    /// Weight for false-positive budget remaining (default 0.10).
    pub false_positive: f64,
    /// Weight for performance budget score (default 0.10).
    pub performance: f64,
}

impl Default for ReplacementWeights {
    fn default() -> Self {
        Self { parity_corpus: 0.50, canary_repos: 0.30, false_positive: 0.10, performance: 0.10 }
    }
}

/// Inputs for computing the weighted replacement score.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct ReplacementInputs {
    /// Parity corpus pass rate (0.0 - 1.0).
    pub parity_score: f64,
    /// Canary repo pass rate (0.0 - 1.0).
    pub canary_score: f64,
    /// False-positive budget remaining (1.0 = no FPs, 0.0 = all FPs).
    pub false_positive_score: f64,
    /// Performance budget score (1.0 = within budget, 0.0 = 5x over budget).
    pub performance_score: f64,
}

/// Compute the weighted replacement score (0-100).
///
/// The formula is:
///   score = (parity * 0.50 + canary * 0.30 + fp * 0.10 + perf * 0.10) * 100
///
/// Each input is expected to be in the range `[0.0, 1.0]`. The final score is
/// clamped to `[0.0, 100.0]`.
pub fn compute_replacement_score(inputs: &ReplacementInputs, weights: &ReplacementWeights) -> f64 {
    #[allow(clippy::suboptimal_flops)]
    let raw = inputs.parity_score * weights.parity_corpus
        + inputs.canary_score * weights.canary_repos
        + inputs.false_positive_score * weights.false_positive
        + inputs.performance_score * weights.performance;
    (raw * 100.0).clamp(0.0, 100.0)
}

// ---------------------------------------------------------------------------
// Per-family tier classification and release gates
// ---------------------------------------------------------------------------

/// Tier classification for a parity family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyTier {
    /// Must reach >= 97% for release.
    Tier1,
    /// Required for 99% overall replacement score.
    Tier2,
}

/// Map a family name to its tier.
///
/// Tier-1 families are the most popular bundlers, test runners, and meta-
/// frameworks whose parity is required for a credible replacement claim.
pub fn family_tier(family: &str) -> FamilyTier {
    match family {
        "vite" | "vitest" | "webpack" | "jest" | "storybook" | "next" | "nuxt" | "astro"
        | "sveltekit" | "remix" | "angular" | "nx" | "playwright" => FamilyTier::Tier1,
        _ => FamilyTier::Tier2,
    }
}

/// Result of checking release gates.
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseGateResult {
    /// Whether all gates passed.
    pub passed: bool,
    /// The computed replacement score (0-100).
    pub replacement_score: f64,
    /// Tier-1 families that scored below the 97% threshold.
    pub tier1_below_threshold: Vec<String>,
    /// False-positive delta percentage.
    pub false_positive_delta: f64,
    /// Cold-scan slowdown percentage.
    pub cold_scan_slowdown_pct: f64,
    /// Speed ratio vs knip (e.g. 5.0 means 5x faster).
    pub speed_ratio_vs_knip: f64,
    /// Human-readable descriptions of each failed gate.
    pub failures: Vec<String>,
}

/// Check release gates against the replacement score and associated metrics.
///
/// Gates:
/// - Replacement score must be >= 99%.
/// - Every Tier-1 family must score >= 97%.
/// - False-positive delta must be <= 2%.
/// - Cold-scan slowdown must be <= 20%.
/// - Speed ratio vs knip must be >= 3x.
pub fn check_release_gates(
    replacement_score: f64,
    family_scores: &[(String, f64, FamilyTier)],
    false_positive_delta: f64,
    cold_scan_slowdown_pct: f64,
    speed_ratio_vs_knip: f64,
) -> ReleaseGateResult {
    let mut failures = Vec::new();
    let mut tier1_below = Vec::new();

    if replacement_score < 99.0 {
        failures.push(format!("replacement score {replacement_score:.1}% < 99% threshold"));
    }

    for (family, score, tier) in family_scores {
        if *tier == FamilyTier::Tier1 && *score < 97.0 {
            tier1_below.push(family.clone());
            failures.push(format!("Tier-1 family '{family}' at {score:.1}% < 97% threshold"));
        }
    }

    if false_positive_delta > 2.0 {
        failures.push(format!("false-positive delta {false_positive_delta:.1}% > 2% threshold"));
    }

    if cold_scan_slowdown_pct > 20.0 {
        failures.push(format!("cold-scan slowdown {cold_scan_slowdown_pct:.1}% > 20% threshold"));
    }

    if speed_ratio_vs_knip < 3.0 {
        failures.push(format!("speed ratio vs knip {speed_ratio_vs_knip:.1}x < 3x threshold"));
    }

    ReleaseGateResult {
        passed: failures.is_empty(),
        replacement_score,
        tier1_below_threshold: tier1_below,
        false_positive_delta,
        cold_scan_slowdown_pct,
        speed_ratio_vs_knip,
        failures,
    }
}

// ---------------------------------------------------------------------------
// Family discovery
// ---------------------------------------------------------------------------

/// Discover all family names present under the corpus root directory.
///
/// This is filesystem-driven: every subdirectory of `corpus_root` is treated
/// as a family. The returned list is sorted alphabetically. Families that
/// exist on disk but have no valid cases inside are still returned, so that
/// CI can detect empty/broken family directories.
pub fn discover_family_names(corpus_root: &Path) -> Vec<String> {
    let mut families = Vec::new();
    let Ok(entries) = std::fs::read_dir(corpus_root) else {
        return families;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|ft| ft.is_dir())
            && let Some(name) = entry.file_name().to_str()
        {
            // Skip hidden directories (e.g. .git, .DS_Store).
            if !name.starts_with('.') {
                families.push(name.to_string());
            }
        }
    }
    families.sort();
    families
}

/// Discover all parity cases under the given corpus root.
///
/// The corpus root is expected to contain family subdirectories, each
/// containing case subdirectories. Each case must have a `meta.json` file;
/// an `expected.json` file is optional (defaults to empty expectations).
///
/// Returns a sorted list of (meta, expected, `case_dir`) tuples.
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
        if !family_entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            continue;
        }
        let Ok(cases_in_family) = std::fs::read_dir(family_entry.path()) else {
            continue;
        };
        for case_entry in cases_in_family.flatten() {
            if !case_entry.file_type().is_ok_and(|ft| ft.is_dir()) {
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
#[allow(clippy::cast_precision_loss)]
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
        let tier = family_tier(&f.family);
        let tier_label = match tier {
            FamilyTier::Tier1 => "[T1]",
            FamilyTier::Tier2 => "[T2]",
        };
        let _ = writeln!(
            out,
            "  {:<30} {}/{} ({:.1}%) {}",
            f.family, f.passed_cases, f.total_cases, f.pct, tier_label
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

/// Format a release gate result as a human-readable report.
pub fn format_release_gate_report(gate: &ReleaseGateResult) -> String {
    let mut out = String::new();

    let status = if gate.passed { "PASSED" } else { "FAILED" };
    let _ = writeln!(out, "Release Gate: {status}");
    let _ = writeln!(out, "  Replacement score:   {:.1}%", gate.replacement_score);
    let _ = writeln!(out, "  FP delta:            {:.1}%", gate.false_positive_delta);
    let _ = writeln!(out, "  Cold-scan slowdown:  {:.1}%", gate.cold_scan_slowdown_pct);
    let _ = writeln!(out, "  Speed vs knip:       {:.1}x", gate.speed_ratio_vs_knip);

    if !gate.tier1_below_threshold.is_empty() {
        let _ = writeln!(out, "  Tier-1 below 97%:    {}", gate.tier1_below_threshold.join(", "));
    }

    if !gate.failures.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Gate failures:");
        for f in &gate.failures {
            let _ = writeln!(out, "  - {f}");
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Canary repo infrastructure
// ---------------------------------------------------------------------------

/// Configuration for a canary repository.
///
/// Each canary repo represents a popular real-world project that exercises one
/// or more Tier-1 framework families. Reference data (FP counts, scan times)
/// are captured from knip runs on the specified git refs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryRepoConfig {
    /// Name of the canary repo (e.g. "calcom", "nextjs-dashboard").
    pub name: String,
    /// Git clone URL or local path.
    pub source: String,
    /// Git ref to checkout (branch, tag, or commit).
    pub git_ref: Option<String>,
    /// Subdirectory within the repo to analyze (for monorepos).
    pub subdir: Option<String>,
    /// Expected false-positive count from reference tool (knip).
    pub reference_fp_count: usize,
    /// Expected finding count from reference tool.
    pub reference_finding_count: usize,
    /// Reference tool cold-scan duration in milliseconds.
    pub reference_cold_scan_ms: u64,
    /// Tier-1 frameworks present in this repo.
    pub frameworks: Vec<String>,
}

/// Result of analyzing a single canary repo.
#[derive(Debug, Clone, Serialize)]
pub struct CanaryRepoResult {
    /// Name of the canary repo.
    pub name: String,
    /// Whether this repo passed all gates (FP delta and speed ratio).
    pub passed: bool,
    /// Total number of findings reported by pruneguard.
    pub finding_count: usize,
    /// Number of false positives in the findings.
    pub false_positive_count: usize,
    /// Percentage change in false positives relative to the reference tool.
    pub false_positive_delta_pct: f64,
    /// Wall-clock cold-scan duration in milliseconds.
    pub cold_scan_ms: u64,
    /// Speed ratio vs reference tool (> 1.0 means faster than reference).
    pub speed_ratio: f64,
    /// Frameworks detected during analysis.
    pub frameworks_detected: Vec<String>,
    /// Any errors encountered during analysis.
    pub errors: Vec<String>,
}

/// Aggregate result across all canary repos.
#[derive(Debug, Clone, Serialize)]
pub struct CanaryAggregateResult {
    /// Total number of canary repos evaluated.
    pub total_repos: usize,
    /// Number of canary repos that passed all gates.
    pub passed_repos: usize,
    /// Pass rate across all canary repos (0.0 - 1.0).
    pub pass_rate: f64,
    /// Average false-positive delta percentage across all repos.
    pub avg_false_positive_delta_pct: f64,
    /// Average speed ratio across all repos.
    pub avg_speed_ratio: f64,
    /// Worst (highest) false-positive delta percentage.
    pub worst_fp_delta_pct: f64,
    /// Worst (lowest) speed ratio.
    pub worst_speed_ratio: f64,
    /// Individual repo results.
    pub repo_results: Vec<CanaryRepoResult>,
}

/// Evaluate canary repo results against reference data.
///
/// This is a pure evaluation function: it takes the analysis results (finding
/// count, FP count, scan time, detected frameworks) and compares them against
/// the reference data in the `CanaryRepoConfig`.
///
/// A canary repo passes when:
/// - The absolute false-positive delta is <= 2%.
/// - The speed ratio is >= 3x (i.e. pruneguard is at least 3x faster).
#[allow(clippy::cast_precision_loss)]
pub fn evaluate_canary_repo(
    config: &CanaryRepoConfig,
    finding_count: usize,
    false_positive_count: usize,
    cold_scan_ms: u64,
    frameworks_detected: Vec<String>,
) -> CanaryRepoResult {
    let fp_delta = if config.reference_fp_count == 0 {
        if false_positive_count == 0 { 0.0 } else { 100.0 }
    } else {
        let delta = false_positive_count as f64 - config.reference_fp_count as f64;
        (delta / config.reference_fp_count as f64) * 100.0
    };

    let speed_ratio = if cold_scan_ms == 0 {
        f64::INFINITY
    } else {
        config.reference_cold_scan_ms as f64 / cold_scan_ms as f64
    };

    let passed = fp_delta.abs() <= 2.0 && speed_ratio >= 3.0;

    CanaryRepoResult {
        name: config.name.clone(),
        passed,
        finding_count,
        false_positive_count,
        false_positive_delta_pct: fp_delta,
        cold_scan_ms,
        speed_ratio,
        frameworks_detected,
        errors: Vec::new(),
    }
}

/// Compute aggregate canary repo metrics from individual results.
///
/// When there are no results, all rates and averages default to zero (except
/// `worst_speed_ratio` which defaults to `INFINITY`).
#[allow(clippy::cast_precision_loss)]
pub fn compute_canary_aggregate(results: &[CanaryRepoResult]) -> CanaryAggregateResult {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let pass_rate = if total == 0 { 0.0 } else { passed as f64 / total as f64 };

    let avg_fp_delta = if total == 0 {
        0.0
    } else {
        results.iter().map(|r| r.false_positive_delta_pct).sum::<f64>() / total as f64
    };
    let avg_speed = if total == 0 {
        0.0
    } else {
        results.iter().map(|r| r.speed_ratio).sum::<f64>() / total as f64
    };

    let worst_fp = results.iter().map(|r| r.false_positive_delta_pct).fold(0.0f64, f64::max);
    let worst_speed = results.iter().map(|r| r.speed_ratio).fold(f64::INFINITY, f64::min);

    CanaryAggregateResult {
        total_repos: total,
        passed_repos: passed,
        pass_rate,
        avg_false_positive_delta_pct: avg_fp_delta,
        avg_speed_ratio: avg_speed,
        worst_fp_delta_pct: worst_fp,
        worst_speed_ratio: worst_speed,
        repo_results: results.to_vec(),
    }
}

/// Default canary repo configurations for CI validation.
///
/// These represent popular real-world projects that exercise the Tier-1
/// framework families. Reference data (FP counts, scan times) are from
/// knip runs on the specified git refs.
pub fn default_canary_repos() -> Vec<CanaryRepoConfig> {
    vec![
        CanaryRepoConfig {
            name: "create-t3-app".to_string(),
            source: "https://github.com/t3-oss/create-t3-app".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 0,
            reference_finding_count: 12,
            reference_cold_scan_ms: 4500,
            frameworks: vec!["next".to_string()],
        },
        CanaryRepoConfig {
            name: "vitesse".to_string(),
            source: "https://github.com/antfu-collective/vitesse".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 0,
            reference_finding_count: 8,
            reference_cold_scan_ms: 3200,
            frameworks: vec!["vite".to_string(), "vitest".to_string()],
        },
        CanaryRepoConfig {
            name: "nuxt-ui".to_string(),
            source: "https://github.com/nuxt/ui".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 2,
            reference_finding_count: 45,
            reference_cold_scan_ms: 8000,
            frameworks: vec!["nuxt".to_string(), "vitest".to_string()],
        },
        CanaryRepoConfig {
            name: "astro-starlight".to_string(),
            source: "https://github.com/withastro/starlight".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 1,
            reference_finding_count: 20,
            reference_cold_scan_ms: 5500,
            frameworks: vec!["astro".to_string(), "vitest".to_string()],
        },
        CanaryRepoConfig {
            name: "shadcn-svelte".to_string(),
            source: "https://github.com/huntabyte/shadcn-svelte".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 0,
            reference_finding_count: 15,
            reference_cold_scan_ms: 3800,
            frameworks: vec!["sveltekit".to_string(), "vitest".to_string()],
        },
        CanaryRepoConfig {
            name: "epic-stack".to_string(),
            source: "https://github.com/epicweb-dev/epic-stack".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 0,
            reference_finding_count: 18,
            reference_cold_scan_ms: 6000,
            frameworks: vec!["remix".to_string(), "vitest".to_string(), "playwright".to_string()],
        },
        CanaryRepoConfig {
            name: "angular-realworld".to_string(),
            source: "https://github.com/gothinkster/angular-realworld-example-app".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 0,
            reference_finding_count: 5,
            reference_cold_scan_ms: 2500,
            frameworks: vec!["angular".to_string()],
        },
        CanaryRepoConfig {
            name: "nx-examples".to_string(),
            source: "https://github.com/nrwl/nx-examples".to_string(),
            git_ref: Some("main".to_string()),
            subdir: None,
            reference_fp_count: 1,
            reference_finding_count: 30,
            reference_cold_scan_ms: 7000,
            frameworks: vec!["nx".to_string(), "jest".to_string(), "storybook".to_string()],
        },
    ]
}

/// Compute replacement inputs from parity scores and canary results.
///
/// Translates the raw parity percentage and canary aggregate metrics into the
/// normalised `[0.0, 1.0]` inputs expected by [`compute_replacement_score`].
///
/// - `parity_score` = parity `overall_pct` / 100.
/// - `canary_score` = canary `pass_rate`.
/// - `false_positive_score` = 1.0 minus the relative FP delta, clamped.
/// - `performance_score` = average speed ratio / 3.0, clamped to 1.0.
pub fn compute_full_replacement_inputs(
    parity_score: &ExternalParityScore,
    canary_result: &CanaryAggregateResult,
) -> ReplacementInputs {
    let parity = parity_score.overall_pct / 100.0;
    let canary = canary_result.pass_rate;

    // FP score: 1.0 = no excess FPs, 0.0 = FPs are 100%+ worse than reference.
    let fp_score = (1.0 - canary_result.avg_false_positive_delta_pct.abs() / 100.0).clamp(0.0, 1.0);

    // Performance score: 1.0 = within budget, scaled down to 0.0 at 5x over.
    let perf_score = if canary_result.avg_speed_ratio >= 3.0 {
        1.0
    } else {
        (canary_result.avg_speed_ratio / 3.0).clamp(0.0, 1.0)
    };

    ReplacementInputs {
        parity_score: parity,
        canary_score: canary,
        false_positive_score: fp_score,
        performance_score: perf_score,
    }
}

/// Format a canary aggregate result as a human-readable report.
pub fn format_canary_report(result: &CanaryAggregateResult) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "Canary Repo Results: {}/{} repos passing ({:.1}%)",
        result.passed_repos,
        result.total_repos,
        result.pass_rate * 100.0
    );
    let _ = writeln!(out, "  Avg FP delta:    {:.1}%", result.avg_false_positive_delta_pct);
    let _ = writeln!(out, "  Avg speed ratio: {:.1}x", result.avg_speed_ratio);
    let _ = writeln!(out, "  Worst FP delta:  {:.1}%", result.worst_fp_delta_pct);
    let _ = writeln!(out, "  Worst speed:     {:.1}x", result.worst_speed_ratio);
    let _ = writeln!(out);

    let _ = writeln!(out, "Individual repos:");
    for r in &result.repo_results {
        let status = if r.passed { "PASS" } else { "FAIL" };
        let _ = writeln!(
            out,
            "  [{status}] {:<25} findings={:<4} FP={:<4} FP-delta={:>+6.1}%  speed={:.1}x  frameworks={}",
            r.name,
            r.finding_count,
            r.false_positive_count,
            r.false_positive_delta_pct,
            r.speed_ratio,
            r.frameworks_detected.join(", ")
        );
        for err in &r.errors {
            let _ = writeln!(out, "    error: {err}");
        }
    }

    out
}

/// Discover canary repo configurations from a directory.
///
/// Looks for `canary.json` files in subdirectories of `canary_root`. Each
/// `canary.json` must deserialise into a `CanaryRepoConfig`. Directories
/// without a `canary.json` are silently skipped.
///
/// The returned list is sorted by name.
pub fn discover_canary_configs(canary_root: &Path) -> Vec<CanaryRepoConfig> {
    let mut configs = Vec::new();
    let Ok(entries) = std::fs::read_dir(canary_root) else {
        return configs;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            continue;
        }
        let config_path = entry.path().join("canary.json");
        let Ok(content) = std::fs::read_to_string(&config_path) else {
            continue;
        };
        let Ok(config): Result<CanaryRepoConfig, _> = serde_json::from_str(&content) else {
            continue;
        };
        configs.push(config);
    }
    configs.sort_by(|a, b| a.name.cmp(&b.name));
    configs
}
