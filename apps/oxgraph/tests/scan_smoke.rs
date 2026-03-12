use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/cases/{name}"))
}

fn run_oxgraph(root: &std::path::Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_oxgraph"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("oxgraph should run");

    serde_json::from_slice(&output.stdout).expect("command should emit valid json")
}

#[test]
fn scan_reports_unused_file_from_fixture() {
    let root = fixture_root("unused-file-basic");
    let report = run_oxgraph(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-file" && finding["subject"] == "src/unused.ts"
    }));
}

#[test]
fn impact_reports_entrypoint_from_fixture() {
    let root = fixture_root("unused-file-basic");
    let report = run_oxgraph(&root, &["--format", "json", "impact", "src/used.ts"]);
    let entrypoints = report["affectedEntrypoints"]
        .as_array()
        .expect("affectedEntryPoints array");

    assert!(entrypoints.iter().any(|entrypoint| {
        entrypoint
            .as_str()
            .is_some_and(|entrypoint| entrypoint.ends_with("src/index.ts"))
    }));
}

#[test]
fn explain_returns_proof_for_reachable_file() {
    let root = fixture_root("unused-file-basic");
    let report = run_oxgraph(&root, &["--format", "json", "explain", "src/used.ts"]);

    assert_eq!(report["matchedNode"].as_str(), Some("src/used.ts"));
    assert!(report["proofs"].as_array().is_some_and(|proofs| !proofs.is_empty()));
}
