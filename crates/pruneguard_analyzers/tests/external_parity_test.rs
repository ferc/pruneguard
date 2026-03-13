use pruneguard_analyzers::external_parity::{
    ParityCaseResult, compute_external_parity_score, discover_parity_cases,
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
    assert!(cases.len() >= 20, "expected at least 20 parity cases, found {}", cases.len());
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
        "config-liveness",
        "dynamic-patterns",
        "export-semantics",
        "member-semantics",
        "source-adapters",
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
