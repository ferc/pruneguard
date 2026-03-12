use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/cases/{name}"))
}

fn temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should advance")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("oxgraph-{prefix}-{unique}"));
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
    run(root, &["git", "config", "user.email", "oxgraph@example.com"]);
    run(root, &["git", "config", "user.name", "oxgraph"]);
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

fn run_oxgraph(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oxgraph"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("oxgraph should run")
}

fn run_oxgraph_json(root: &Path, args: &[&str]) -> Value {
    let output = run_oxgraph(root, args);
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "oxgraph failed:\nstdout:\n{}\nstderr:\n{}",
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

    let report = run_oxgraph_json(
        &root,
        &["--format", "json", "--changed-since", "HEAD~1", "scan"],
    );
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

    let report = run_oxgraph_json(
        &root,
        &[
            "--format",
            "json",
            "--changed-since",
            "HEAD~1",
            "--no-cache",
            "scan",
        ],
    );
    let findings = report["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|finding| finding["subject"] == "src/unused-b.ts"));
    assert_eq!(report["stats"]["affectedScopeIncomplete"].as_bool(), Some(true));
}

#[test]
fn baseline_suppresses_existing_findings() {
    let root = temp_dir("baseline");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_oxgraph_json(&root, &["--format", "json", "scan"]);
    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&first).expect("serialize baseline"),
    )
    .expect("baseline write");

    let second = run_oxgraph_json(&root, &["--format", "json", "scan"]);
    assert_eq!(
        second["findings"].as_array().map_or(usize::MAX, Vec::len),
        0
    );
    assert_eq!(second["stats"]["baselineApplied"].as_bool(), Some(true));
    assert!(second["stats"]["suppressedFindings"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn warm_cache_reuses_file_facts() {
    let root = temp_dir("cache");
    copy_tree(&fixture_root("unused-file-basic"), &root);

    let first = run_oxgraph_json(&root, &["--format", "json", "scan"]);
    let second = run_oxgraph_json(&root, &["--format", "json", "scan"]);

    assert_eq!(first["summary"]["totalFiles"], second["summary"]["totalFiles"]);
    assert!(second["stats"]["filesCached"].as_u64().unwrap_or(0) > 0);
    assert!(second["stats"]["cacheHits"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn baseline_profile_mismatch_is_reported() {
    let root = temp_dir("baseline-mismatch");
    copy_tree(&fixture_root("unused-dependency-prod-dev"), &root);

    let development = run_oxgraph_json(
        &root,
        &["--format", "json", "--profile", "development", "scan"],
    );
    fs::write(
        root.join("baseline.json"),
        serde_json::to_vec_pretty(&development).expect("serialize baseline"),
    )
    .expect("baseline write");

    let production = run_oxgraph_json(
        &root,
        &["--format", "json", "--profile", "production", "scan"],
    );
    assert_eq!(
        production["stats"]["baselineProfileMismatch"].as_bool(),
        Some(true)
    );
}

#[test]
fn changed_since_tracks_renamed_files() {
    let root = temp_dir("changed-since-rename");
    copy_tree(&fixture_root("unused-file-basic"), &root);
    init_git_repo(&root);

    run(
        &root,
        &[
            "git",
            "mv",
            "src/unused.ts",
            "src/renamed-unused.ts",
        ],
    );
    run(&root, &["git", "commit", "-am", "rename unused"]);

    let report = run_oxgraph_json(
        &root,
        &["--format", "json", "--changed-since", "HEAD~1", "scan"],
    );
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

    let _ = run_oxgraph_json(&root, &["--format", "json", "scan"]);

    fs::remove_file(root.join("src/unused.ts")).expect("delete file");
    run(&root, &["git", "add", "-A"]);
    run(&root, &["git", "commit", "-m", "delete unused"]);

    let report = run_oxgraph_json(
        &root,
        &["--format", "json", "--changed-since", "HEAD~1", "scan"],
    );
    assert_eq!(report["stats"]["affectedScopeIncomplete"].as_bool(), Some(false));
}

#[test]
fn scan_dot_outputs_graphviz() {
    let root = fixture_root("unused-file-basic");
    let output = run_oxgraph(&root, &["--format", "dot", "scan"]);
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph oxgraph"));
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
    let report = run_oxgraph_json(
        &root,
        &[
            "--format",
            "json",
            "migrate",
            "knip",
            knip_config.to_string_lossy().as_ref(),
        ],
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
    let report = run_oxgraph_json(
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
