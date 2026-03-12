use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/cases/{name}"))
}

fn run_pruneguard(root: &std::path::Path, args: &[&str]) -> Value {
    let mut argv = vec!["--no-cache"];
    argv.extend_from_slice(args);
    let output = Command::new(env!("CARGO_BIN_EXE_pruneguard"))
        .current_dir(root)
        .args(&argv)
        .output()
        .expect("pruneguard should run");

    serde_json::from_slice(&output.stdout).expect("command should emit valid json")
}

#[test]
fn scan_reports_unused_file_from_fixture() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-file" && finding["subject"] == "src/unused.ts"
    }));
}

#[test]
fn partial_scope_scans_are_marked_advisory() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan", "src/used.ts"]);

    assert_eq!(report["stats"]["partialScope"].as_bool(), Some(true));
    assert!(
        report["stats"]["partialScopeReason"]
            .as_str()
            .is_some_and(|reason| reason.contains("partial-scope"))
    );
}

#[test]
fn dead_code_findings_include_confidence() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-file"
            && finding["subject"] == "src/unused.ts"
            && finding["confidence"] == "high"
    }));
}

#[test]
fn impact_reports_entrypoint_from_fixture() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "impact", "src/used.ts"]);
    let entrypoints = report["affectedEntrypoints"].as_array().expect("affectedEntryPoints array");

    assert!(entrypoints.iter().any(|entrypoint| {
        entrypoint.as_str().is_some_and(|entrypoint| entrypoint.ends_with("src/index.ts"))
    }));
}

#[test]
fn explain_returns_proof_for_reachable_file() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "explain", "src/used.ts"]);

    assert_eq!(report["matchedNode"].as_str(), Some("src/used.ts"));
    assert_eq!(report["queryKind"].as_str(), Some("file"));
    assert!(report["proofs"].as_array().is_some_and(|proofs| !proofs.is_empty()));
}

#[test]
fn reexports_star_only_keeps_consumed_exports_live() {
    let root = fixture_root("reexports-star");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/leaf.ts#unused"
    }));
    assert!(!findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/leaf.ts#used"
    }));
}

#[test]
fn type_only_liveness_respects_type_consumers() {
    let root = fixture_root("type-only-liveness");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(!findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/types.ts#Foo"
    }));
    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/types.ts#Bar"
    }));
    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/types.ts#runtimeOnly"
    }));
}

#[test]
fn fixture_files_are_in_inventory_but_excluded_from_dead_code_findings() {
    let root = fixture_root("fixtures-excluded-by-default");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    let files = report["inventories"]["files"].as_array().expect("files array");

    assert!(
        files
            .iter()
            .any(|file| { file["path"] == "fixtures/helper.ts" && file["role"] == "fixture" })
    );
    assert!(!findings.iter().any(|finding| {
        finding["code"] == "unused-file" && finding["subject"] == "fixtures/helper.ts"
    }));
}

#[test]
fn package_scripts_are_detected_as_entrypoints_with_sources() {
    let root = fixture_root("package-scripts-roots");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let entrypoints = report["entrypoints"].as_array().expect("entrypoints array");

    assert!(
        entrypoints
            .iter()
            .any(|entrypoint| { entrypoint["source"] == "package-script:build:scripts/build.ts" })
    );
    assert!(
        entrypoints
            .iter()
            .any(|entrypoint| { entrypoint["source"] == "package-script:dev:scripts/dev.ts" })
    );
}

#[test]
fn ownership_reports_cross_owner_edges() {
    let root = fixture_root("ownership-cross-owner");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "ownership-cross-owner"
            && finding["subject"] == "src/index.ts -> src/b.ts"
    }));
}

#[test]
fn focus_filters_findings_without_changing_inventory() {
    let root = fixture_root("focus-filtering");
    let report = run_pruneguard(&root, &["--format", "json", "--focus", "src/used.ts", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    let files = report["inventories"]["files"].as_array().expect("files array");

    assert_eq!(report["stats"]["focusApplied"].as_bool(), Some(true));
    assert_eq!(report["stats"]["focusedFiles"].as_u64(), Some(1));
    assert!(files.iter().any(|file| file["path"] == "src/unused.ts"));
    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/used.ts#extra"
    }));
    assert!(!findings.iter().any(|finding| { finding["subject"] == "src/unused.ts" }));
}

#[test]
fn explain_focus_reports_filtered_related_output() {
    let root = fixture_root("focus-filtering");
    let report = run_pruneguard(
        &root,
        &["--format", "json", "--focus", "src/used.ts", "explain", "src/unused.ts"],
    );

    assert_eq!(report["matchedNode"].as_str(), Some("src/unused.ts"));
    assert_eq!(report["queryKind"].as_str(), Some("file"));
    assert_eq!(report["focusFiltered"].as_bool(), Some(true));
}

#[test]
fn namespace_imports_only_keep_accessed_members_live() {
    let root = fixture_root("namespace-imports");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(!findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/leaf.ts#used"
    }));
    assert!(findings.iter().any(|finding| {
        finding["code"] == "unused-export" && finding["subject"] == "src/leaf.ts#unused"
    }));
}

#[test]
fn package_fields_are_detected_as_entrypoints() {
    let root = fixture_root("entrypoints-package-fields");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let entrypoints = report["entrypoints"].as_array().expect("entrypoints array");

    assert!(entrypoints.iter().any(|entrypoint| { entrypoint["source"] == "package:src/main.ts" }));
    assert!(
        entrypoints.iter().any(|entrypoint| { entrypoint["source"] == "package:./src/public.ts" })
    );
    assert!(entrypoints.iter().any(|entrypoint| { entrypoint["kind"] == "package-bin" }));
}

#[test]
fn tsconfig_path_aliases_resolve_without_unresolved_edges() {
    let root = fixture_root("tsconfig-paths-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    assert_eq!(report["stats"]["unresolvedSpecifiers"].as_u64(), Some(0));
}

#[test]
fn boundaries_path_filters_report_violations() {
    let root = fixture_root("boundaries-path");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "boundary-violation"
            && finding["ruleName"] == "no-internal"
            && finding["subject"] == "src/app.ts -> src/internal/secret.ts"
    }));
}

#[test]
fn boundaries_workspace_filters_report_violations() {
    let root = fixture_root("boundaries-workspace");
    let report = run_pruneguard(&root, &["--format", "json", "--max-findings", "20", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    assert!(
        findings.iter().any(|finding| {
            finding["ruleName"] == "workspace-boundary"
                && finding["message"].as_str().is_some_and(|message| message.contains("@fixture/b"))
        }),
        "expected workspace boundary violation"
    );
}

#[test]
fn boundaries_package_filters_report_violations() {
    let root = fixture_root("boundaries-package");
    let report = run_pruneguard(&root, &["--format", "json", "--max-findings", "20", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    assert!(
        findings.iter().any(|finding| {
            finding["ruleName"] == "package-boundary"
                && finding["message"].as_str().is_some_and(|message| message.contains("@fixture/b"))
        }),
        "expected package boundary violation"
    );
}

#[test]
fn boundary_rules_support_tags_and_tag_not() {
    let root = fixture_root("rules-tags");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["ruleName"] == "frontend-no-platform"
            && finding["subject"] == "src/app.ts -> src/platform.ts"
    }));
    assert!(
        !findings
            .iter()
            .any(|finding| { finding["subject"] == "src/internal/allowed.ts -> src/platform.ts" })
    );
}

#[test]
fn boundary_rules_support_reachable_from() {
    let root = fixture_root("rules-reachable-from");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["ruleName"] == "reachable-from-index"
            && finding["subject"] == "src/app.ts -> src/secret.ts"
    }));
}

#[test]
fn boundary_rules_support_reaches() {
    let root = fixture_root("rules-reaches");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["ruleName"] == "app-reaches-leaf"
            && finding["subject"] == "src/app.ts -> src/mid.ts"
    }));
}

#[test]
fn boundaries_dependency_kind_filters_match_dynamic_imports() {
    let root = fixture_root("boundaries-dependency-kinds");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| { finding["ruleName"] == "no-dynamic-internal" }));
}

#[test]
fn boundaries_profile_filters_only_match_development_roots() {
    let root = fixture_root("boundaries-profiles");
    let dev_report = run_pruneguard(&root, &["--format", "json", "--profile", "development", "scan"]);
    let prod_report = run_pruneguard(&root, &["--format", "json", "--profile", "production", "scan"]);

    assert!(dev_report["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| finding["ruleName"] == "dev-cannot-hit-internal")
    }));
    assert!(!prod_report["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| finding["ruleName"] == "dev-cannot-hit-internal")
    }));
}

#[test]
fn boundaries_entrypoint_kind_filters_match_script_entrypoints() {
    let root = fixture_root("boundaries-entrypoint-kinds");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    assert!(report["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| finding["ruleName"] == "script-cannot-hit-internal")
    }));
}

#[test]
fn ownership_team_config_overrides_codeowners_when_explicitly_matched() {
    let root = fixture_root("ownership-codeowners-precedence");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(findings.iter().any(|finding| {
        finding["code"] == "ownership-cross-owner"
            && finding["subject"] == "src/index.ts -> src/special.ts"
    }));
}

#[test]
fn unused_dependencies_split_production_and_development_usage() {
    let root = fixture_root("unused-dependency-prod-dev");
    let prod_report = run_pruneguard(&root, &["--format", "json", "--profile", "production", "scan"]);
    let dev_report = run_pruneguard(&root, &["--format", "json", "--profile", "development", "scan"]);

    assert!(!prod_report["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| finding["subject"] == "left-pad")
    }));
    assert!(
        prod_report["findings"].as_array().is_some_and(|findings| findings
            .iter()
            .all(|finding| { finding["subject"] != "vite" }))
    );
    assert!(
        !dev_report["findings"].as_array().is_some_and(|findings| findings
            .iter()
            .any(|finding| { finding["subject"] == "vite" }))
    );
}

#[test]
fn cycles_include_candidate_break_edges() {
    let root = fixture_root("cycles-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    let cycle = findings.iter().find(|finding| finding["code"] == "cycle").expect("cycle finding");
    let evidence = cycle["evidence"].as_array().expect("evidence array");

    assert!(evidence.iter().any(|item| {
        item["description"]
            .as_str()
            .is_some_and(|description| description.contains("Candidate break edges"))
    }));
}
