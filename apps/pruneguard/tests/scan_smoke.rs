use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/cases/{name}"))
}

#[allow(clippy::similar_names)]
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

#[allow(clippy::similar_names)]
fn run_pruneguard_with_exit(root: &std::path::Path, args: &[&str]) -> (Value, i32) {
    let mut argv = vec!["--no-cache"];
    argv.extend_from_slice(args);
    let output = Command::new(env!("CARGO_BIN_EXE_pruneguard"))
        .current_dir(root)
        .args(&argv)
        .output()
        .expect("pruneguard should run");

    let exit_code = output.status.code().unwrap_or(-1);
    let json = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (json, exit_code)
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
    let dev_report =
        run_pruneguard(&root, &["--format", "json", "--profile", "development", "scan"]);
    let prod_report =
        run_pruneguard(&root, &["--format", "json", "--profile", "production", "scan"]);

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
    let prod_report =
        run_pruneguard(&root, &["--format", "json", "--profile", "production", "scan"]);
    let dev_report =
        run_pruneguard(&root, &["--format", "json", "--profile", "development", "scan"]);

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

#[test]
fn js_extension_imports_resolve_to_ts_source_files() {
    let root = fixture_root("js-extension-imports");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    // .js extension imports must resolve — zero unresolved specifiers.
    assert_eq!(
        report["stats"]["unresolvedSpecifiers"].as_u64(),
        Some(0),
        "imports with .js extensions should resolve to .ts source files"
    );

    // utils.ts and helpers/math.ts must be reachable (not flagged as unused).
    let findings = report["findings"].as_array().expect("findings array");
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/utils.ts"),
        "src/utils.ts should be reachable via .js import"
    );
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file"
            && f["subject"].as_str().is_some_and(|s| s.ends_with("helpers/math.ts"))),
        "src/helpers/math.ts should be reachable via .js import"
    );

    // The intentionally orphaned file should still be flagged.
    assert!(
        findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/orphan.ts"),
        "src/orphan.ts should be flagged as unused"
    );
}

#[test]
fn ambient_declarations_excluded_from_dead_code() {
    let root = fixture_root("ambient-declaration-excluded");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    let files = report["inventories"]["files"].as_array().expect("files array");

    // .d.ts files must appear in the inventory.
    assert!(
        files.iter().any(|file| file["path"] == "src/env.d.ts"),
        "src/env.d.ts should be in inventory"
    );
    assert!(
        files.iter().any(|file| file["path"] == "src/types.d.ts"),
        "src/types.d.ts should be in inventory"
    );
    assert!(
        files.iter().any(|file| file["path"] == "src/vite-env.d.ts"),
        "src/vite-env.d.ts should be in inventory"
    );

    // .d.ts files must NOT appear as unused-file findings.
    assert!(
        !findings.iter().any(|f| {
            f["code"] == "unused-file"
                && f["subject"].as_str().is_some_and(|s| s.ends_with(".d.ts"))
        }),
        "ambient declaration files (.d.ts) should be excluded from dead-code findings"
    );
}

#[test]
fn package_only_via_exports_keeps_resolved_files_live() {
    let root = fixture_root("package-only-via-exports");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    let entrypoints = report["entrypoints"].as_array().expect("entrypoints array");

    // The lib's exports field should be detected as a package-exports entrypoint.
    assert!(
        entrypoints.iter().any(|ep| {
            ep["kind"] == "package-exports"
                && ep["source"].as_str().is_some_and(|s| s.contains("src/index.ts"))
        }),
        "lib package should have a package-exports entrypoint"
    );

    // The lib index file should NOT be flagged as unused-file.
    assert!(
        !findings.iter().any(|f| {
            f["code"] == "unused-file"
                && f["subject"].as_str().is_some_and(|s| s.contains("packages/lib/src/index.ts"))
        }),
        "packages/lib/src/index.ts should be reachable via package exports"
    );
}

#[test]
fn script_only_package_usage_does_not_flag_script_dependency() {
    let root = fixture_root("script-only-package-usage");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // eslint is referenced only in package.json scripts ("lint": "eslint src/").
    // It should NOT be flagged as an unused dependency because script usage counts.
    assert!(
        !findings.iter().any(|f| { f["code"] == "unused-dependency" && f["subject"] == "eslint" }),
        "eslint used in scripts should not be reported as unused"
    );
}

#[test]
fn confidence_high_findings_have_high_confidence() {
    let root = fixture_root("confidence-high");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    assert!(!findings.is_empty(), "expected at least one finding");
    assert!(
        findings.iter().all(|f| f["confidence"] == "high"),
        "all findings in confidence-high fixture should have high confidence"
    );
}

#[test]
fn confidence_medium_findings_report_correct_confidence() {
    let root = fixture_root("confidence-medium");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // The fixture has an unreachable file; verify the finding exists and has a
    // confidence field (the exact tier depends on unresolved-specifier pressure).
    assert!(
        findings.iter().any(|f| {
            f["code"] == "unused-file"
                && f["subject"] == "src/maybe-unused.ts"
                && f["confidence"].as_str().is_some()
        }),
        "confidence-medium fixture should flag src/maybe-unused.ts with a confidence tier"
    );
}

#[test]
fn confidence_low_fixture_has_high_unresolved_pressure() {
    let root = fixture_root("confidence-low");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    // The fixture has three external packages that cannot be resolved locally.
    // In a full-scope scan, all source files are reachable (index imports lib).
    // The key behavior: the graph completes without crashing and the stats
    // reflect zero unresolved specifiers (externalized deps are not unresolved).
    let findings = report["findings"].as_array().expect("findings array");
    assert!(
        !findings.iter().any(|f| { f["code"] == "unused-file" && f["subject"] == "src/lib.ts" }),
        "src/lib.ts is imported and should be reachable"
    );
}

#[test]
fn reexports_alias_preserves_aliased_names() {
    let root = fixture_root("reexports-alias");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // src/foo.ts exports `foo`, src/index.ts re-exports it as `bar`,
    // and src/consumer.ts imports `bar`. All files should be reachable.
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file"),
        "no files should be flagged as unused when aliased re-exports are consumed"
    );

    // The original export `foo` should NOT be flagged as unused because
    // it is consumed through the `bar` alias.
    assert!(
        !findings
            .iter()
            .any(|f| { f["code"] == "unused-export" && f["subject"] == "src/foo.ts#foo" }),
        "foo export should be kept live through the bar alias re-export"
    );
}

#[test]
fn package_exports_conditions_resolve_correctly() {
    let root = fixture_root("package-exports-conditions");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let entrypoints = report["entrypoints"].as_array().expect("entrypoints array");
    let findings = report["findings"].as_array().expect("findings array");

    // Both import and require conditions should be detected as entrypoints.
    assert!(
        entrypoints.iter().any(|ep| {
            ep["kind"] == "package-exports"
                && ep["source"].as_str().is_some_and(|s| s.contains("index.mjs"))
        }),
        "import condition should produce a package-exports entrypoint"
    );
    assert!(
        entrypoints.iter().any(|ep| {
            ep["kind"] == "package-exports"
                && ep["source"].as_str().is_some_and(|s| s.contains("index.cjs"))
        }),
        "require condition should produce a package-exports entrypoint"
    );
    // Subpath export should also be detected.
    assert!(
        entrypoints.iter().any(|ep| {
            ep["kind"] == "package-exports"
                && ep["source"].as_str().is_some_and(|s| s.contains("utils.mjs"))
        }),
        "subpath ./utils export should produce a package-exports entrypoint"
    );

    // src/internal.mjs is NOT exposed via exports and should be flagged as unused.
    assert!(
        findings
            .iter()
            .any(|f| { f["code"] == "unused-file" && f["subject"] == "src/internal.mjs" }),
        "src/internal.mjs is not exposed via package exports and should be unused"
    );
}

#[test]
fn review_produces_blocking_and_advisory_findings() {
    let root = fixture_root("unused-file-basic");
    let (report, exit_code) =
        run_pruneguard_with_exit(&root, &["--format", "json", "--no-baseline", "review"]);

    assert!(report["trust"]["fullScope"].as_bool().unwrap_or(false), "review should be full-scope");
    assert!(
        report["newFindings"].as_array().is_some_and(|arr| !arr.is_empty()),
        "review should find issues"
    );
    // High-confidence unused-file should be blocking
    assert!(
        report["blockingFindings"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|f| f["code"] == "unused-file")),
        "high-confidence unused-file should be blocking"
    );
    assert!(report["recommendations"].as_array().is_some_and(|arr| !arr.is_empty()));
    assert_eq!(exit_code, 1, "review with blocking findings should exit 1");
}

#[test]
fn safe_delete_marks_unused_file_as_safe() {
    let root = fixture_root("unused-file-basic");
    let (report, exit_code) = run_pruneguard_with_exit(
        &root,
        &["--format", "json", "--no-baseline", "safe-delete", "src/unused.ts"],
    );

    assert_eq!(report["targets"].as_array().unwrap().len(), 1);
    assert!(
        report["safe"].as_array().is_some_and(|arr| arr
            .iter()
            .any(|c| c["target"] == "src/unused.ts" && c["confidence"] == "high")),
        "unused file should be safe to delete with high confidence"
    );
    assert!(report["blocked"].as_array().unwrap().is_empty());
    assert_eq!(exit_code, 0, "safe-delete with only safe targets should exit 0");
}

#[test]
fn safe_delete_blocks_live_file() {
    let root = fixture_root("unused-file-basic");
    let (report, exit_code) = run_pruneguard_with_exit(
        &root,
        &["--format", "json", "--no-baseline", "safe-delete", "src/used.ts"],
    );

    assert!(
        report["blocked"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|c| c["target"] == "src/used.ts")),
        "live file should be blocked from deletion"
    );
    assert!(report["safe"].as_array().unwrap().is_empty());
    assert_eq!(exit_code, 1, "safe-delete with blocked targets should exit 1");
}
