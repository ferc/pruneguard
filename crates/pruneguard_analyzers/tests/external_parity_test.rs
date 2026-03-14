use pruneguard_analyzers::external_parity::{
    CanaryRepoConfig, ParityCaseResult, compute_canary_aggregate, compute_external_parity_score,
    compute_full_replacement_inputs, default_canary_repos, discover_canary_configs,
    discover_parity_cases, evaluate_canary_repo, format_canary_report,
    format_external_parity_report,
};
use std::path::PathBuf;

fn corpus_root() -> PathBuf {
    // Navigate from the crate root to the project-level fixtures directory.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    PathBuf::from(manifest_dir).join("..").join("..").join("fixtures").join("parity")
}

#[test]
fn discover_finds_all_cases() {
    let root = corpus_root();
    let cases = discover_parity_cases(&root);
    assert!(cases.len() >= 70, "expected at least 70 parity cases, found {}", cases.len());
    eprintln!("Discovered {} parity cases:", cases.len());
    for (meta, _expected, dir) in &cases {
        eprintln!("  {}/{} (ref: {}) at {:?}", meta.family, meta.name, meta.reference_tool, dir);
    }
}

#[test]
fn all_cases_have_valid_meta() {
    let root = corpus_root();
    let cases = discover_parity_cases(&root);

    for (meta, _expected, _dir) in &cases {
        assert!(!meta.family.is_empty(), "family must not be empty");
        assert!(!meta.name.is_empty(), "name must not be empty");
        assert!(!meta.reference_tool.is_empty(), "reference_tool must not be empty");
        assert!(!meta.description.is_empty(), "description must not be empty");
    }
}

#[test]
fn cases_are_sorted_by_family_and_name() {
    let root = corpus_root();
    let cases = discover_parity_cases(&root);

    for window in cases.windows(2) {
        let (a_meta, _, _) = &window[0];
        let (b_meta, _, _) = &window[1];
        let order = a_meta.family.cmp(&b_meta.family).then(a_meta.name.cmp(&b_meta.name));
        assert!(
            order != std::cmp::Ordering::Greater,
            "cases not sorted: {}/{} should come before {}/{}",
            a_meta.family,
            a_meta.name,
            b_meta.family,
            b_meta.name
        );
    }
}

#[test]
fn expected_families_are_present() {
    let root = corpus_root();
    let cases = discover_parity_cases(&root);

    let families: Vec<String> = cases
        .iter()
        .map(|(meta, _, _)| meta.family.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let required_families = [
        "angular",
        "astro",
        "config-liveness",
        "dynamic-patterns",
        "export-semantics",
        "jest",
        "member-semantics",
        "next",
        "nuxt",
        "nx",
        "playwright",
        "remix",
        "source-adapters",
        "storybook",
        "sveltekit",
        "vite",
        "vitest",
        "webpack",
    ];

    for family in &required_families {
        assert!(families.contains(&family.to_string()), "missing expected family: {family}");
    }
}

#[test]
fn scoring_with_mock_results() {
    let results = vec![
        ParityCaseResult {
            family: "dynamic-patterns".to_string(),
            name: "require-resolve".to_string(),
            reference_tool: "dependency-cruiser".to_string(),
            passed: true,
            total_checks: 2,
            passed_checks: 2,
            failures: vec![],
        },
        ParityCaseResult {
            family: "dynamic-patterns".to_string(),
            name: "import-meta-glob-literal".to_string(),
            reference_tool: "knip".to_string(),
            passed: false,
            total_checks: 3,
            passed_checks: 1,
            failures: vec![
                "reachable ./src/modules/a.ts not found in graph".to_string(),
                "reachable ./src/modules/b.ts not found in graph".to_string(),
            ],
        },
        ParityCaseResult {
            family: "source-adapters".to_string(),
            name: "vue-sfc-script".to_string(),
            reference_tool: "dependency-cruiser".to_string(),
            passed: true,
            total_checks: 1,
            passed_checks: 1,
            failures: vec![],
        },
    ];

    let score = compute_external_parity_score(&results);

    assert_eq!(score.total_cases, 3);
    assert_eq!(score.passed_cases, 2);
    assert_eq!(score.total_checks, 6);
    assert_eq!(score.passed_checks, 4);
    assert!((score.overall_pct - 66.666).abs() < 1.0, "expected ~66.7%, got {}", score.overall_pct);

    // By family.
    assert_eq!(score.by_family.len(), 2);
    let dp = score.by_family.iter().find(|f| f.family == "dynamic-patterns").unwrap();
    assert_eq!(dp.total_cases, 2);
    assert_eq!(dp.passed_cases, 1);

    let sa = score.by_family.iter().find(|f| f.family == "source-adapters").unwrap();
    assert_eq!(sa.total_cases, 1);
    assert_eq!(sa.passed_cases, 1);

    // By reference tool.
    assert_eq!(score.by_reference_tool.len(), 2);
    let dc = score.by_reference_tool.iter().find(|t| t.tool == "dependency-cruiser").unwrap();
    assert_eq!(dc.total_cases, 2);
    assert_eq!(dc.passed_cases, 2);

    let knip = score.by_reference_tool.iter().find(|t| t.tool == "knip").unwrap();
    assert_eq!(knip.total_cases, 1);
    assert_eq!(knip.passed_cases, 0);
}

#[test]
fn scoring_with_all_passing() {
    let results = vec![ParityCaseResult {
        family: "a".to_string(),
        name: "x".to_string(),
        reference_tool: "knip".to_string(),
        passed: true,
        total_checks: 5,
        passed_checks: 5,
        failures: vec![],
    }];

    let score = compute_external_parity_score(&results);
    assert_eq!(score.overall_pct, 100.0);
    assert_eq!(score.passed_cases, 1);
}

#[test]
fn scoring_with_empty_results() {
    let results: Vec<ParityCaseResult> = vec![];
    let score = compute_external_parity_score(&results);
    assert_eq!(score.total_cases, 0);
    assert_eq!(score.overall_pct, 0.0);
}

#[test]
fn report_formatting() {
    let results = vec![
        ParityCaseResult {
            family: "dynamic-patterns".to_string(),
            name: "require-resolve".to_string(),
            reference_tool: "dependency-cruiser".to_string(),
            passed: true,
            total_checks: 2,
            passed_checks: 2,
            failures: vec![],
        },
        ParityCaseResult {
            family: "dynamic-patterns".to_string(),
            name: "import-meta-glob-literal".to_string(),
            reference_tool: "knip".to_string(),
            passed: false,
            total_checks: 3,
            passed_checks: 1,
            failures: vec!["file not found".to_string()],
        },
    ];

    let score = compute_external_parity_score(&results);
    let report = format_external_parity_report(&score);

    assert!(report.contains("External Parity Score:"), "report should have title");
    assert!(report.contains("By family:"), "report should have family section");
    assert!(report.contains("By reference tool:"), "report should have tool section");
    assert!(report.contains("Failed cases:"), "report should have failed section");
    assert!(report.contains("dynamic-patterns"), "report should mention the family");
    assert!(report.contains("file not found"), "report should include failure detail");

    eprintln!("\n{report}");
}

// ---------------------------------------------------------------------------
// Canary repo tests
// ---------------------------------------------------------------------------

fn make_canary_config(
    name: &str,
    ref_fp: usize,
    ref_findings: usize,
    ref_ms: u64,
) -> CanaryRepoConfig {
    CanaryRepoConfig {
        name: name.to_string(),
        source: format!("https://example.com/{name}"),
        git_ref: Some("main".to_string()),
        subdir: None,
        reference_fp_count: ref_fp,
        reference_finding_count: ref_findings,
        reference_cold_scan_ms: ref_ms,
        frameworks: vec!["next".to_string()],
    }
}

#[test]
fn canary_evaluation_passing() {
    let config = make_canary_config("test-app", 0, 12, 4500);
    let result = evaluate_canary_repo(&config, 12, 0, 1000, vec!["next".to_string()]);

    assert!(result.passed, "should pass: 0 FP delta and speed ratio 4.5x");
    assert_eq!(result.false_positive_delta_pct, 0.0);
    assert!((result.speed_ratio - 4.5).abs() < 0.01, "expected 4.5x, got {}", result.speed_ratio);
    assert_eq!(result.finding_count, 12);
    assert!(result.errors.is_empty());
}

#[test]
fn canary_evaluation_fp_over_budget() {
    let config = make_canary_config("fp-heavy", 5, 20, 6000);
    let result = evaluate_canary_repo(&config, 20, 10, 1000, vec![]);

    assert!(!result.passed, "should fail: FP delta 100% > 2%");
    assert!((result.false_positive_delta_pct - 100.0).abs() < 0.01);
    // Speed is fine (6.0x), but FP delta busts the budget.
    assert!(result.speed_ratio >= 3.0);
}

#[test]
fn canary_evaluation_too_slow() {
    let config = make_canary_config("slow-scan", 0, 10, 3000);
    // scan_ms = 2000 => speed_ratio = 1.5x < 3.0x
    let result = evaluate_canary_repo(&config, 10, 0, 2000, vec![]);

    assert!(!result.passed, "should fail: speed ratio 1.5x < 3x");
    assert!((result.speed_ratio - 1.5).abs() < 0.01);
    assert_eq!(result.false_positive_delta_pct, 0.0);
}

#[test]
fn canary_evaluation_zero_ref_fp_with_new_fps() {
    let config = make_canary_config("zero-ref", 0, 10, 3000);
    let result = evaluate_canary_repo(&config, 10, 3, 500, vec![]);

    // When reference FP count is 0 but we have FPs, delta = 100%.
    assert!(!result.passed);
    assert!((result.false_positive_delta_pct - 100.0).abs() < 0.01);
}

#[test]
fn canary_evaluation_zero_scan_time() {
    let config = make_canary_config("instant", 0, 5, 3000);
    let result = evaluate_canary_repo(&config, 5, 0, 0, vec![]);

    // Zero scan time means infinite speed ratio.
    assert!(result.speed_ratio.is_infinite());
    assert!(result.passed);
}

#[test]
fn canary_aggregate_computation() {
    let config_a = make_canary_config("repo-a", 0, 10, 4500);
    let config_b = make_canary_config("repo-b", 2, 20, 6000);
    let config_c = make_canary_config("repo-c", 0, 5, 3000);

    let result_a = evaluate_canary_repo(&config_a, 10, 0, 1000, vec!["next".to_string()]);
    let result_b = evaluate_canary_repo(&config_b, 20, 2, 1500, vec!["nuxt".to_string()]);
    let result_c = evaluate_canary_repo(&config_c, 5, 5, 2000, vec![]); // fails: FP delta = 100%

    let agg = compute_canary_aggregate(&[result_a, result_b, result_c]);

    assert_eq!(agg.total_repos, 3);
    assert_eq!(agg.passed_repos, 2); // repo-a and repo-b pass; repo-c fails
    assert!((agg.pass_rate - 2.0 / 3.0).abs() < 0.01);

    // Worst FP delta should be 100% (repo-c).
    assert!((agg.worst_fp_delta_pct - 100.0).abs() < 0.01);

    // Worst speed ratio should be the lowest (repo-c at 1.5x).
    assert!((agg.worst_speed_ratio - 1.5).abs() < 0.01);
}

#[test]
fn canary_aggregate_empty() {
    let agg = compute_canary_aggregate(&[]);
    assert_eq!(agg.total_repos, 0);
    assert_eq!(agg.passed_repos, 0);
    assert_eq!(agg.pass_rate, 0.0);
    assert_eq!(agg.avg_false_positive_delta_pct, 0.0);
    assert_eq!(agg.avg_speed_ratio, 0.0);
}

#[test]
fn full_replacement_inputs_computation() {
    // Simulate a parity score of 80%.
    let parity = compute_external_parity_score(&[
        ParityCaseResult {
            family: "vite".to_string(),
            name: "basic".to_string(),
            reference_tool: "knip".to_string(),
            passed: true,
            total_checks: 8,
            passed_checks: 8,
            failures: vec![],
        },
        ParityCaseResult {
            family: "vite".to_string(),
            name: "advanced".to_string(),
            reference_tool: "knip".to_string(),
            passed: false,
            total_checks: 2,
            passed_checks: 0,
            failures: vec!["missed file".to_string()],
        },
    ]);

    let config = make_canary_config("test", 10, 20, 6000);
    let result = evaluate_canary_repo(&config, 20, 10, 1000, vec!["next".to_string()]);
    let agg = compute_canary_aggregate(&[result]);

    let inputs = compute_full_replacement_inputs(&parity, &agg);

    // Parity: 8/10 checks = 80% => 0.8.
    assert!((inputs.parity_score - 0.8).abs() < 0.01);

    // Canary: 0/1 passed (FP delta = 0% so pass) => 0.0
    // Actually FP delta is 0.0% since (10 - 10)/10 = 0.0, and speed = 6.0x
    // So the single result passes, pass_rate = 1.0.
    assert!((inputs.canary_score - 1.0).abs() < 0.01);

    // FP score: avg FP delta = 0.0% => 1.0 - 0.0 = 1.0.
    assert!((inputs.false_positive_score - 1.0).abs() < 0.01);

    // Performance score: speed_ratio = 6.0x >= 3.0 => 1.0.
    assert!((inputs.performance_score - 1.0).abs() < 0.01);
}

#[test]
fn full_replacement_inputs_with_poor_canary() {
    let parity = compute_external_parity_score(&[ParityCaseResult {
        family: "a".to_string(),
        name: "x".to_string(),
        reference_tool: "knip".to_string(),
        passed: true,
        total_checks: 10,
        passed_checks: 10,
        failures: vec![],
    }]);

    let config = make_canary_config("slow-repo", 0, 10, 3000);
    // FP = 5 (delta 100%), scan = 2000ms (speed 1.5x) => fails both.
    let result = evaluate_canary_repo(&config, 10, 5, 2000, vec![]);
    let agg = compute_canary_aggregate(&[result]);

    let inputs = compute_full_replacement_inputs(&parity, &agg);

    assert!((inputs.parity_score - 1.0).abs() < 0.01);
    assert!((inputs.canary_score - 0.0).abs() < 0.01); // 0/1 passed
    assert!((inputs.false_positive_score - 0.0).abs() < 0.01); // delta = 100%
    assert!((inputs.performance_score - 0.5).abs() < 0.01); // 1.5/3.0 = 0.5
}

#[test]
fn default_canary_repos_are_valid() {
    let repos = default_canary_repos();
    assert!(repos.len() >= 5, "expected at least 5 canary repos, got {}", repos.len());
    for repo in &repos {
        assert!(!repo.name.is_empty(), "canary repo name must not be empty");
        assert!(!repo.source.is_empty(), "canary repo source must not be empty");
        assert!(!repo.frameworks.is_empty(), "canary repo must have at least one framework");
        assert!(
            repo.reference_cold_scan_ms > 0,
            "canary repo '{}' must have a positive reference scan time",
            repo.name
        );
    }
}

#[test]
fn default_canary_repos_cover_tier1_families() {
    let repos = default_canary_repos();
    let all_frameworks: Vec<String> = repos
        .iter()
        .flat_map(|r| r.frameworks.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    // Verify coverage of important Tier-1 families.
    let expected =
        ["next", "nuxt", "vite", "vitest", "astro", "sveltekit", "remix", "angular", "nx"];
    for family in &expected {
        assert!(
            all_frameworks.contains(&family.to_string()),
            "default canary repos should cover Tier-1 family '{family}'"
        );
    }
}

#[test]
fn canary_report_formatting() {
    let config = make_canary_config("test-app", 0, 12, 4500);
    let result = evaluate_canary_repo(&config, 12, 0, 1000, vec!["next".to_string()]);
    let agg = compute_canary_aggregate(&[result]);
    let report = format_canary_report(&agg);

    assert!(report.contains("Canary Repo Results:"), "report should have a title");
    assert!(report.contains("1/1 repos passing"), "report should show pass count");
    assert!(report.contains("[PASS]"), "report should show PASS status");
    assert!(report.contains("test-app"), "report should include repo name");
    assert!(report.contains("Avg FP delta:"), "report should show avg FP delta");
    assert!(report.contains("Avg speed ratio:"), "report should show avg speed ratio");

    eprintln!("\n{report}");
}

#[test]
fn discover_canary_configs_empty_dir() {
    let dir = std::env::temp_dir().join("pruneguard_canary_test_empty");
    let _ = std::fs::create_dir_all(&dir);
    let configs = discover_canary_configs(&dir);
    assert!(configs.is_empty(), "empty directory should yield no configs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discover_canary_configs_with_json() {
    let dir = std::env::temp_dir().join("pruneguard_canary_test_json");
    let sub = dir.join("my-repo");
    let _ = std::fs::create_dir_all(&sub);

    let config_json = serde_json::json!({
        "name": "my-repo",
        "source": "https://example.com/my-repo",
        "git_ref": "main",
        "subdir": null,
        "reference_fp_count": 0,
        "reference_finding_count": 10,
        "reference_cold_scan_ms": 3000,
        "frameworks": ["vite"]
    });
    std::fs::write(sub.join("canary.json"), config_json.to_string()).unwrap();

    let configs = discover_canary_configs(&dir);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, "my-repo");
    assert_eq!(configs[0].frameworks, vec!["vite"]);

    let _ = std::fs::remove_dir_all(&dir);
}
