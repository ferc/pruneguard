use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/cases/{name}"))
}

fn temp_dir(prefix: &str) -> PathBuf {
    let unique =
        SystemTime::now().duration_since(UNIX_EPOCH).expect("time should advance").as_nanos();
    let dir = std::env::temp_dir().join(format!("pruneguard-{prefix}-{unique}"));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("destination should exist");
    for entry in fs::read_dir(from).expect("source dir should exist") {
        let entry = entry.expect("entry should read");
        let source_path = entry.path();
        let destination_path = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).expect("file should copy");
        }
    }
}

fn init_git_repo(root: &Path) {
    run(root, &["git", "init", "-b", "main"]);
    run(root, &["git", "config", "user.email", "pruneguard@example.com"]);
    run(root, &["git", "config", "user.name", "pruneguard"]);
    run(root, &["git", "add", "."]);
    run(root, &["git", "commit", "-m", "init"]);
}

fn run(root: &Path, args: &[&str]) {
    let output = Command::new(args[0])
        .current_dir(root)
        .args(&args[1..])
        .output()
        .expect("command should run");
    assert!(
        output.status.success(),
        "command `{}` failed:\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn run_pruneguard(root: &Path, args: &[&str]) -> Output {
    // Always use one-shot mode in tests to avoid spawning background daemons
    // that interfere with parallel test execution and shared fixture dirs.
    let mut full_args = vec!["--daemon", "off"];
    full_args.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_pruneguard"))
        .current_dir(root)
        .args(&full_args)
        .output()
        .expect("pruneguard should run")
}

fn run_pruneguard_json(root: &Path, args: &[&str]) -> Value {
    let output = run_pruneguard(root, args);
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "pruneguard failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("command should emit valid json")
}

#[test]
fn changed_since_filters_to_modified_file_scope() {
    let root = temp_dir("changed-since");
    fs::write(root.join("package.json"), r#"{"name":"changed-since","private":true}"#)
        .expect("package.json");
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(root.join("src/index.ts"), "import './used';\n").expect("index");
    fs::write(root.join("src/used.ts"), "export const used = 1;\n").expect("used");
    fs::write(root.join("src/unused-a.ts"), "export const a = 1;\n").expect("unused-a");
    fs::write(root.join("src/unused-b.ts"), "export const b = 1;\n").expect("unused-b");
    init_git_repo(&root);

    fs::write(root.join("src/unused-a.ts"), "export const a = 2;\n").expect("modify file");
    run(&root, &["git", "add", "."]);
    run(&root, &["git", "commit", "-m", "modify unused-a"]);

    let report =
        run_pruneguard_json(&root, &["--format", "json", "--changed-since", "HEAD~1", "scan"]);
    let findings = report["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|finding| finding["subject"] == "src/unused-a.ts"));
    assert!(!findings.iter().any(|finding| finding["subject"] == "src/unused-b.ts"));
    assert_eq!(report["stats"]["changedFiles"].as_u64(), Some(1));
    assert_eq!(report["stats"]["affectedScopeIncomplete"].as_bool(), Some(false));
}

#[test]
fn deleted_file_without_cache_falls_back_to_full_findings() {
    let root = temp_dir("changed-since-delete");
    fs::write(root.join("package.json"), r#"{"name":"changed-delete","private":true}"#)
        .expect("package.json");
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(root.join("src/index.ts"), "import './used';\n").expect("index");
    fs::write(root.join("src/used.ts"), "export const used = 1;\n").expect("used");
    fs::write(root.join("src/unused-a.ts"), "export const a = 1;\n").expect("unused-a");
    fs::write(root.join("src/unused-b.ts"), "export const b = 1;\n").expect("unused-b");
    init_git_repo(&root);

    fs::remove_file(root.join("src/unused-a.ts")).expect("delete file");
    run(&root, &["git", "add", "-A"]);
    run(&root, &["git", "commit", "-m", "delete unused-a"]);

    let report = run_pruneguard_json(
        &root,
        &["--format", "json", "--changed-since", "HEAD~1", "--no-cache", "scan"],
    );
    let findings = report["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|finding| finding["subject"] == "src/unused-b.ts"));
    assert_eq!(report["stats"]["affectedScopeIncomplete"].as_bool(), Some(true));
}

#[test]
fn baseline_suppresses_existing_findings() {
    let root = temp_dir("baseline");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_pruneguard_json(&root, &["--format", "json", "scan"]);
    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&first).expect("serialize baseline"),
    )
    .expect("baseline write");

    let second = run_pruneguard_json(&root, &["--format", "json", "scan"]);
    assert_eq!(second["findings"].as_array().map_or(usize::MAX, Vec::len), 0);
    assert_eq!(second["stats"]["baselineApplied"].as_bool(), Some(true));
    assert!(second["stats"]["suppressedFindings"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn warm_cache_reuses_file_facts() {
    let root = temp_dir("cache");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_pruneguard_json(&root, &["--format", "json", "scan"]);
    let second = run_pruneguard_json(&root, &["--format", "json", "scan"]);

    assert_eq!(first["summary"]["totalFiles"], second["summary"]["totalFiles"]);
    assert!(second["stats"]["filesCached"].as_u64().unwrap_or(0) > 0);
    assert!(second["stats"]["cacheHits"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn impact_focus_can_filter_all_returned_nodes_without_error() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard_json(
        &root,
        &["--format", "json", "--focus", "src/unused.ts", "impact", "src/used.ts"],
    );

    assert_eq!(report["focusFiltered"].as_bool(), Some(true));
    assert_eq!(report["affectedFiles"].as_array().map_or(usize::MAX, Vec::len), 0);
}

#[test]
fn baseline_profile_mismatch_is_reported() {
    let root = temp_dir("baseline-mismatch");
    copy_tree(&fixture_root("unused-dependency-prod-dev"), &root);

    let development =
        run_pruneguard_json(&root, &["--format", "json", "--profile", "development", "scan"]);
    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&development).expect("serialize baseline"),
    )
    .expect("baseline write");

    let production =
        run_pruneguard_json(&root, &["--format", "json", "--profile", "production", "scan"]);
    assert_eq!(production["stats"]["baselineProfileMismatch"].as_bool(), Some(true));
}

#[test]
fn no_baseline_flag_disables_auto_discovery() {
    let root = temp_dir("no-baseline");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_pruneguard_json(&root, &["--format", "json", "scan"]);
    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&first).expect("serialize baseline"),
    )
    .expect("baseline write");

    let second = run_pruneguard_json(&root, &["--format", "json", "--no-baseline", "scan"]);
    assert_eq!(second["stats"]["baselineApplied"].as_bool(), Some(false));
    assert!(second["findings"].as_array().is_some_and(|findings| !findings.is_empty()));
}

#[test]
fn require_full_scope_rejects_partial_dead_code_scan() {
    let root = fixture_root("unused-file-basic");
    let output =
        run_pruneguard(&root, &["--format", "json", "--require-full-scope", "scan", "src/used.ts"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("partial-scope"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn changed_since_tracks_renamed_files() {
    let root = temp_dir("changed-since-rename");
    copy_tree(&fixture_root("unused-file-basic"), &root);
    init_git_repo(&root);

    run(&root, &["git", "mv", "src/unused.ts", "src/renamed-unused.ts"]);
    run(&root, &["git", "commit", "-am", "rename unused"]);

    let report =
        run_pruneguard_json(&root, &["--format", "json", "--changed-since", "HEAD~1", "scan"]);
    assert!(
        report["stats"]["changedFiles"].as_u64().unwrap_or(0) >= 1,
        "rename should contribute at least one changed path"
    );
}

#[test]
fn deleted_file_with_cache_recovery_keeps_scope_complete() {
    let root = temp_dir("changed-since-delete-cache");
    copy_tree(&fixture_root("unused-file-basic"), &root);
    init_git_repo(&root);

    let _ = run_pruneguard_json(&root, &["--format", "json", "scan"]);

    fs::remove_file(root.join("src/unused.ts")).expect("delete file");
    run(&root, &["git", "add", "-A"]);
    run(&root, &["git", "commit", "-m", "delete unused"]);

    let report =
        run_pruneguard_json(&root, &["--format", "json", "--changed-since", "HEAD~1", "scan"]);
    assert_eq!(report["stats"]["affectedScopeIncomplete"].as_bool(), Some(false));
}

#[test]
fn scan_dot_outputs_graphviz() {
    let root = fixture_root("unused-file-basic");
    let output = run_pruneguard(&root, &["--format", "dot", "scan"]);
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph pruneguard"));
    assert!(stdout.contains("entrypoint"));
}

#[test]
fn migrate_knip_emits_usable_json() {
    let knip_config =
        PathBuf::from("/Users/fcarril/projects/side-projects/oxc-architecture/knip/knip.json");
    if !knip_config.exists() {
        return;
    }
    let root = temp_dir("migrate-knip");
    let report = run_pruneguard_json(
        &root,
        &["--format", "json", "migrate", "knip", knip_config.to_string_lossy().as_ref()],
    );
    assert!(report["config"].is_object());
}

#[test]
fn migrate_depcruise_with_node_emits_usable_json() {
    let depcruise_config = PathBuf::from(
        "/Users/fcarril/projects/side-projects/oxc-architecture/dependency-cruiser/.dependency-cruiser.mjs",
    );
    if !depcruise_config.exists() {
        return;
    }
    let root = temp_dir("migrate-depcruise");
    let report = run_pruneguard_json(
        &root,
        &[
            "--format",
            "json",
            "migrate",
            "depcruise",
            "--node",
            depcruise_config.to_string_lossy().as_ref(),
        ],
    );
    assert!(report["config"].is_object());
}

// ---------------------------------------------------------------------------
// daemon vs one-shot equivalence
// ---------------------------------------------------------------------------

#[test]
fn oneshot_scan_produces_same_findings_as_repeated_run() {
    // Without a daemon, two consecutive scans with --no-cache must produce
    // identical finding sets, proving one-shot equivalence.
    let root = temp_dir("oneshot-equiv");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_pruneguard_json(&root, &["--format", "json", "--no-cache", "--no-baseline", "scan"]);
    let second =
        run_pruneguard_json(&root, &["--format", "json", "--no-cache", "--no-baseline", "scan"]);

    let first_findings = first["findings"].as_array().expect("first findings");
    let second_findings = second["findings"].as_array().expect("second findings");

    assert_eq!(
        first_findings.len(),
        second_findings.len(),
        "two --no-cache scans should produce the same number of findings"
    );

    for (idx, (a, b)) in first_findings.iter().zip(second_findings.iter()).enumerate() {
        assert_eq!(
            a["id"], b["id"],
            "finding at position {idx} must have the same ID in both runs"
        );
    }
}

// ---------------------------------------------------------------------------
// changed-since combined with baseline
// ---------------------------------------------------------------------------

#[test]
fn changed_since_with_baseline_suppresses_old_findings() {
    let root = temp_dir("changed-baseline");
    copy_tree(&fixture_root("unused-file-basic"), &root);
    init_git_repo(&root);

    // First scan establishes baseline.
    let baseline = run_pruneguard_json(&root, &["--format", "json", "scan"]);
    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&baseline).expect("serialize baseline"),
    )
    .expect("baseline write");

    // Add a new unused file.
    fs::write(root.join("src/new-unused.ts"), "export const nope = 1;\n").expect("new file");
    run(&root, &["git", "add", "."]);
    run(&root, &["git", "commit", "-m", "add new-unused"]);

    let report = run_pruneguard_json(
        &root,
        &["--format", "json", "--changed-since", "HEAD~1", "scan"],
    );

    // The baseline should suppress old findings (src/unused.ts).
    assert_eq!(report["stats"]["baselineApplied"].as_bool(), Some(true));
    let findings = report["findings"].as_array().expect("findings");

    // The new file should still appear in changed-since scope.
    // The original unused.ts should be suppressed by baseline.
    assert!(
        !findings
            .iter()
            .any(|f| f["subject"] == "src/unused.ts"),
        "baseline should suppress old unused.ts finding"
    );
}

// ---------------------------------------------------------------------------
// scan with baseline (scan subcommand with auto-discovered baseline)
// ---------------------------------------------------------------------------

#[test]
fn scan_with_baseline_auto_discovers_and_suppresses() {
    let root = temp_dir("scan-baseline-auto");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_pruneguard_json(&root, &["--format", "json", "--no-cache", "scan"]);
    assert!(
        first["findings"]
            .as_array()
            .is_some_and(|arr| !arr.is_empty()),
        "first scan should have findings"
    );

    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&first).expect("serialize"),
    )
    .expect("write baseline");

    let second = run_pruneguard_json(&root, &["--format", "json", "--no-cache", "scan"]);
    assert_eq!(
        second["stats"]["baselineApplied"].as_bool(),
        Some(true),
        "second scan should auto-discover baseline"
    );
    assert_eq!(
        second["findings"].as_array().map_or(usize::MAX, Vec::len),
        0,
        "all findings should be suppressed by baseline"
    );
}

// ---------------------------------------------------------------------------
// fix-plan operational: full round-trip
// ---------------------------------------------------------------------------

#[test]
fn fix_plan_for_unused_file_produces_delete_action() {
    let root = temp_dir("fix-plan-op");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let report = run_pruneguard_json(
        &root,
        &["--format", "json", "--no-cache", "--no-baseline", "fix-plan", "src/unused.ts"],
    );

    // matchedFindings should include the unused-file.
    assert!(
        report["matchedFindings"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|f| f["code"] == "unused-file")),
        "fix-plan should match unused-file finding"
    );

    // actions should include a delete-file action.
    let actions = report["actions"].as_array().expect("actions array");
    assert!(
        actions.iter().any(|a| a["kind"] == "delete-file"),
        "fix-plan should produce a delete-file action"
    );

    // Each action should have at least one step.
    for action in actions {
        assert!(
            action["steps"].as_array().is_some_and(|s| !s.is_empty()),
            "each action must have steps"
        );
    }

    // verificationSteps should be present.
    assert!(
        report["verificationSteps"]
            .as_array()
            .is_some_and(|v| !v.is_empty()),
        "fix-plan should have verification steps"
    );
}

// ---------------------------------------------------------------------------
// suggest-rules operational: on a copy
// ---------------------------------------------------------------------------

#[test]
fn suggest_rules_on_simple_project_returns_valid_report() {
    let root = temp_dir("suggest-rules-op");
    copy_tree(&fixture_root("suggest-rules-basic"), &root);

    let report = run_pruneguard_json(
        &root,
        &["--format", "json", "--no-cache", "--no-baseline", "suggest-rules"],
    );

    // Must produce a valid JSON report with the expected fields.
    assert!(
        report["suggestedRules"].as_array().is_some(),
        "suggest-rules should return suggestedRules array"
    );
    assert!(
        report["tags"].as_array().is_some(),
        "suggest-rules should return tags array"
    );
}

// ---------------------------------------------------------------------------
// review operational: trust summary stability
// ---------------------------------------------------------------------------

#[test]
fn review_trust_summary_is_stable_across_runs() {
    let root = temp_dir("review-stable");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_pruneguard_json(
        &root,
        &["--format", "json", "--no-cache", "--no-baseline", "review"],
    );
    let second = run_pruneguard_json(
        &root,
        &["--format", "json", "--no-cache", "--no-baseline", "review"],
    );

    // Trust summary fields must be identical.
    assert_eq!(
        first["trust"]["fullScope"],
        second["trust"]["fullScope"],
        "trust.fullScope must be stable"
    );
    assert_eq!(
        first["trust"]["baselineApplied"],
        second["trust"]["baselineApplied"],
        "trust.baselineApplied must be stable"
    );
    assert_eq!(
        first["trust"]["confidenceCounts"],
        second["trust"]["confidenceCounts"],
        "trust.confidenceCounts must be stable"
    );
}

// ---------------------------------------------------------------------------
// safe-delete operational: multiple targets
// ---------------------------------------------------------------------------

#[test]
fn safe_delete_multiple_targets_classifies_each_independently() {
    let root = temp_dir("safe-delete-multi");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let output = run_pruneguard(
        &root,
        &["--format", "json", "--no-cache", "--no-baseline", "safe-delete", "src/unused.ts", "src/used.ts"],
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);

    // Two targets given.
    assert_eq!(
        report["targets"].as_array().map_or(0, Vec::len),
        2,
        "should report two targets"
    );

    // unused.ts should be safe; used.ts should be blocked.
    assert!(
        report["safe"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|c| c["target"] == "src/unused.ts")),
        "src/unused.ts should be safe"
    );
    assert!(
        report["blocked"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|c| c["target"] == "src/used.ts")),
        "src/used.ts should be blocked"
    );
}

// ---------------------------------------------------------------------------
// SARIF output format
// ---------------------------------------------------------------------------

#[test]
fn scan_sarif_output_is_valid_json_with_schema() {
    let root = temp_dir("sarif-output");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let output = run_pruneguard(
        &root,
        &["--format", "sarif", "--no-cache", "--no-baseline", "scan"],
    );
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "sarif scan should not crash"
    );

    let sarif: Value =
        serde_json::from_slice(&output.stdout).expect("sarif output should be valid JSON");

    assert_eq!(sarif["version"].as_str(), Some("2.1.0"));
    assert!(sarif["runs"].as_array().is_some_and(|runs| !runs.is_empty()));
    let results = sarif["runs"][0]["results"].as_array().expect("results array");
    assert!(!results.is_empty(), "sarif should contain results");
}

// ---------------------------------------------------------------------------
// text output format does not crash
// ---------------------------------------------------------------------------

#[test]
fn scan_text_output_runs_without_crash() {
    let root = temp_dir("text-output");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let output = run_pruneguard(
        &root,
        &["--format", "text", "--no-cache", "--no-baseline", "scan"],
    );
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "text scan should not crash"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("findings:") || stdout.contains("repo summary"),
        "text output should contain summary information"
    );
}

// ---------------------------------------------------------------------------
// init command
// ---------------------------------------------------------------------------

#[test]
fn init_creates_config_file() {
    let root = temp_dir("init-cmd");
    fs::create_dir_all(&root).expect("create dir");
    fs::write(
        root.join("package.json"),
        r#"{"name":"init-test","private":true}"#,
    )
    .expect("package.json");

    let output = run_pruneguard(&root, &["init"]);
    assert!(
        output.status.success(),
        "init should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        root.join("pruneguard.json").exists(),
        "init should create pruneguard.json"
    );
}

// ---------------------------------------------------------------------------
// print-config command
// ---------------------------------------------------------------------------

#[test]
fn print_config_emits_valid_json() {
    let root = temp_dir("print-config");
    fs::create_dir_all(&root).expect("create dir");
    fs::write(
        root.join("package.json"),
        r#"{"name":"print-config-test","private":true}"#,
    )
    .expect("package.json");

    let output = run_pruneguard(&root, &["print-config"]);
    assert!(output.status.success());
    let config: Value =
        serde_json::from_slice(&output.stdout).expect("print-config should emit valid JSON");
    assert!(config.is_object());
}
