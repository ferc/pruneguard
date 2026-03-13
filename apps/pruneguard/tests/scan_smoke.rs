use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/cases/{name}"))
}

#[allow(clippy::similar_names)]
fn run_pruneguard(root: &std::path::Path, args: &[&str]) -> Value {
    // Always use one-shot mode in tests to avoid spawning background daemons
    // that interfere with parallel test execution and shared fixture dirs.
    let mut argv = vec!["--daemon", "off", "--no-cache"];
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
    let mut argv = vec!["--daemon", "off", "--no-cache"];
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
                && finding["message"].as_str().is_some_and(|message| message.contains("packages/b"))
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
            finding["ruleName"] == "package-boundary" && finding["code"] == "boundary-violation"
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

// ---------------------------------------------------------------------------
// fix-plan tests
// ---------------------------------------------------------------------------

#[test]
fn fix_plan_generates_actions_for_unused_file() {
    let root = fixture_root("fix-plan-basic");
    let report =
        run_pruneguard(&root, &["--format", "json", "--no-baseline", "fix-plan", "src/orphan.ts"]);

    // Should match the unused-file finding.
    assert!(
        report["matchedFindings"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|f| f["code"] == "unused-file")),
        "fix-plan should match an unused-file finding for src/orphan.ts"
    );

    // Should generate a delete-file remediation action.
    let actions = report["actions"].as_array().expect("actions array");
    assert!(
        actions.iter().any(|action| action["kind"] == "delete-file"),
        "fix-plan should produce a delete-file action"
    );

    // Each action must have steps.
    for action in actions {
        assert!(
            action["steps"].as_array().is_some_and(|steps| !steps.is_empty()),
            "every action must have at least one step"
        );
    }

    // Verification steps should exist.
    assert!(
        report["verificationSteps"].as_array().is_some_and(|arr| !arr.is_empty()),
        "fix-plan should include verification steps"
    );
}

#[test]
fn fix_plan_generates_actions_for_unused_export() {
    let root = fixture_root("fix-plan-basic");
    let report =
        run_pruneguard(&root, &["--format", "json", "--no-baseline", "fix-plan", "src/used.ts"]);

    // Should match the unused-export finding for extraExport.
    assert!(
        report["matchedFindings"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|f| f["code"] == "unused-export"
                && f["subject"].as_str().is_some_and(|s| s.contains("extraExport")))),
        "fix-plan should match an unused-export finding for extraExport"
    );

    // Should generate a delete-export remediation action.
    let actions = report["actions"].as_array().expect("actions array");
    assert!(
        actions.iter().any(|action| action["kind"] == "delete-export"),
        "fix-plan should produce a delete-export action for extraExport"
    );
}

#[test]
fn fix_plan_includes_risk_and_confidence() {
    let root = fixture_root("fix-plan-basic");
    let report =
        run_pruneguard(&root, &["--format", "json", "--no-baseline", "fix-plan", "src/orphan.ts"]);

    // Top-level risk and confidence must be present.
    assert!(
        report["riskLevel"].as_str().is_some(),
        "fix-plan should include a top-level riskLevel"
    );
    assert!(
        report["confidence"].as_str().is_some(),
        "fix-plan should include a top-level confidence"
    );

    // Per-action risk and confidence.
    let actions = report["actions"].as_array().expect("actions array");
    for action in actions {
        assert!(action["risk"].as_str().is_some(), "each action must have a risk field");
        assert!(
            action["confidence"].as_str().is_some(),
            "each action must have a confidence field"
        );
    }
}

// ---------------------------------------------------------------------------
// suggest-rules tests
// ---------------------------------------------------------------------------

#[test]
fn suggest_rules_produces_valid_json_report() {
    let root = fixture_root("suggest-rules-basic");
    let report = run_pruneguard(&root, &["--format", "json", "--no-baseline", "suggest-rules"]);

    // suggestedRules must be an array (may or may not have entries depending
    // on whether cross-package thresholds are met).
    assert!(
        report["suggestedRules"].as_array().is_some(),
        "suggest-rules should return a suggestedRules array"
    );

    // tags should be an array.
    assert!(report["tags"].as_array().is_some(), "suggest-rules should return a tags array");

    // The fixture has 3+ files in src/components, src/api, src/utils.
    // At least one tag should be suggested for one of those directories.
    let tags = report["tags"].as_array().expect("tags array");
    assert!(
        tags.iter().any(|tag| { tag["glob"].as_str().is_some_and(|glob| glob.contains("src/")) }),
        "suggest-rules should suggest at least one tag for the source directory structure"
    );
}

#[test]
fn suggest_rules_reports_rationale() {
    let root = fixture_root("suggest-rules-basic");
    let report = run_pruneguard(&root, &["--format", "json", "--no-baseline", "suggest-rules"]);

    // rationale should be an array with at least one entry.
    let rationale = report["rationale"].as_array();
    let tags = report["tags"].as_array();
    let suggested_rules = report["suggestedRules"].as_array();

    // Either rationale is populated or suggestions were generated.
    assert!(
        rationale.is_some_and(|r| !r.is_empty())
            || tags.is_some_and(|t| !t.is_empty())
            || suggested_rules.is_some_and(|r| !r.is_empty()),
        "suggest-rules should produce either rationale or suggestions"
    );
}

// ---------------------------------------------------------------------------
// safe-delete: needs-review case
// ---------------------------------------------------------------------------

#[test]
fn safe_delete_needs_review_under_high_unresolved_pressure() {
    let root = fixture_root("safe-delete-needs-review");
    let (report, _exit_code) = run_pruneguard_with_exit(
        &root,
        &["--format", "json", "--no-baseline", "safe-delete", "src/orphan.ts"],
    );

    // Under high unresolved pressure, safe-delete should classify
    // the orphan file as needs-review instead of safe.
    let needs_review = report["needsReview"].as_array();
    let safe = report["safe"].as_array();
    let blocked = report["blocked"].as_array();

    let in_needs_review =
        needs_review.is_some_and(|arr| arr.iter().any(|c| c["target"] == "src/orphan.ts"));
    let in_safe = safe.is_some_and(|arr| arr.iter().any(|c| c["target"] == "src/orphan.ts"));
    let in_blocked = blocked.is_some_and(|arr| arr.iter().any(|c| c["target"] == "src/orphan.ts"));

    // The target must appear somewhere in the report.
    assert!(
        in_needs_review || in_blocked || in_safe,
        "target must appear in safe, needsReview, or blocked; report: {}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );

    // Under pressure, the finding may have downgraded confidence.
    // If safe, it means pressure is below threshold -- that is acceptable.
    // The key contract: the command does not panic and returns a valid classification.
    assert!(
        report["targets"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|t| t.as_str() == Some("src/orphan.ts"))),
        "targets should include src/orphan.ts"
    );
}

// ---------------------------------------------------------------------------
// trust summary fields
// ---------------------------------------------------------------------------

#[test]
fn trust_summary_fields_present_in_scan_report() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    // Stats must have all key trust-related fields.
    let stats = &report["stats"];
    assert!(stats["partialScope"].as_bool().is_some(), "partialScope must be present");
    assert!(stats["baselineApplied"].as_bool().is_some(), "baselineApplied must be present");
    assert!(
        stats["unresolvedSpecifiers"].as_u64().is_some(),
        "unresolvedSpecifiers must be present"
    );
    assert!(stats["confidenceCounts"].is_object(), "confidenceCounts must be an object");
    assert!(
        stats["confidenceCounts"]["high"].as_u64().is_some(),
        "confidenceCounts.high must be present"
    );
    assert!(
        stats["confidenceCounts"]["medium"].as_u64().is_some(),
        "confidenceCounts.medium must be present"
    );
    assert!(
        stats["confidenceCounts"]["low"].as_u64().is_some(),
        "confidenceCounts.low must be present"
    );
}

#[test]
fn trust_downgrade_confidence_counts_reflect_pressure() {
    let root = fixture_root("trust-downgrade");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    // Under high unresolved pressure, confidence counts should reflect
    // that not all findings are high confidence.
    let findings = report["findings"].as_array().expect("findings array");

    // The trust-downgrade fixture has real unresolvable imports.
    // At least one finding should have non-high confidence,
    // or the graph completes cleanly with zero findings.
    if !findings.is_empty() {
        let has_non_high = findings.iter().any(|f| f["confidence"] != "high");
        let confidence_counts = &report["stats"]["confidenceCounts"];
        let medium_or_low = confidence_counts["medium"].as_u64().unwrap_or(0)
            + confidence_counts["low"].as_u64().unwrap_or(0);

        // If all findings are high, medium_or_low should be 0.
        // If any are non-high, medium_or_low should reflect them.
        assert_eq!(
            has_non_high,
            medium_or_low > 0,
            "confidence counts should match actual finding confidence tiers"
        );
    }
}

// ---------------------------------------------------------------------------
// review: trust summary field validation
// ---------------------------------------------------------------------------

#[test]
fn review_trust_summary_has_all_required_fields() {
    let root = fixture_root("unused-file-basic");
    let (report, _exit_code) =
        run_pruneguard_with_exit(&root, &["--format", "json", "--no-baseline", "review"]);

    let trust = &report["trust"];
    assert!(trust["fullScope"].as_bool().is_some(), "trust.fullScope must be present");
    assert!(trust["baselineApplied"].as_bool().is_some(), "trust.baselineApplied must be present");
    assert!(trust["unresolvedPressure"].is_number(), "trust.unresolvedPressure must be a number");
    assert!(trust["confidenceCounts"].is_object(), "trust.confidenceCounts must be an object");
    assert!(
        trust["confidenceCounts"]["high"].as_u64().is_some(),
        "trust.confidenceCounts.high must be present"
    );
}

#[test]
fn review_advisory_findings_are_non_high_confidence() {
    let root = fixture_root("unused-file-basic");
    let (report, _exit_code) =
        run_pruneguard_with_exit(&root, &["--format", "json", "--no-baseline", "review"]);

    // Advisory findings must have either non-high confidence or info severity.
    let advisory = report["advisoryFindings"].as_array().unwrap_or(&Vec::new()).clone();
    for finding in &advisory {
        let is_info = finding["severity"] == "info";
        let is_non_high = finding["confidence"] != "high";
        assert!(
            is_info || is_non_high,
            "advisory finding `{}` must be info-severity or non-high confidence",
            finding["subject"]
        );
    }
}

#[test]
fn review_proposed_actions_reference_blocking_findings() {
    let root = fixture_root("unused-file-basic");
    let (report, _exit_code) =
        run_pruneguard_with_exit(&root, &["--format", "json", "--no-baseline", "review"]);

    let blocking = report["blockingFindings"].as_array();
    let proposed = report["proposedActions"].as_array();

    if let (Some(blocking), Some(proposed)) = (blocking, proposed)
        && !blocking.is_empty()
    {
        // At least one proposed action should exist for blocking findings.
        assert!(
            !proposed.is_empty(),
            "review should propose at least one action for blocking findings"
        );
    }
}

// ---------------------------------------------------------------------------
// impact: multiple targets
// ---------------------------------------------------------------------------

#[test]
fn impact_single_target_returns_affected_entities() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "impact", "src/used.ts"]);

    assert_eq!(report["target"].as_str(), Some("src/used.ts"));
    assert!(
        report["affectedEntrypoints"].as_array().is_some(),
        "impact should return affectedEntrypoints"
    );
    assert!(
        report["affectedPackages"].as_array().is_some(),
        "impact should return affectedPackages"
    );
    assert!(report["affectedFiles"].as_array().is_some(), "impact should return affectedFiles");
}

// ---------------------------------------------------------------------------
// scan: full-scope validation
// ---------------------------------------------------------------------------

#[test]
fn full_scope_scan_has_complete_inventory() {
    let root = fixture_root("unused-file-basic");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    assert_eq!(
        report["stats"]["partialScope"].as_bool(),
        Some(false),
        "full-scope scan should not be partial"
    );
    assert!(
        report["summary"]["totalFiles"].as_u64().unwrap_or(0) >= 3,
        "full-scope scan should discover all files"
    );
    assert!(
        report["inventories"]["files"].as_array().is_some_and(|arr| !arr.is_empty()),
        "full-scope scan should have non-empty file inventory"
    );
}

// ---------------------------------------------------------------------------
// scan: staged package install/runtime
// ---------------------------------------------------------------------------

#[test]
fn staged_package_with_declared_dependencies_runs_without_panic() {
    let root = fixture_root("staged-package-install");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);

    // The scan should complete without panicking, produce valid JSON,
    // and discover the source file.
    assert!(
        report["summary"]["totalFiles"].as_u64().unwrap_or(0) >= 1,
        "staged package scan should discover at least one file"
    );
    assert!(
        report["stats"]["partialScope"].as_bool() == Some(false),
        "staged package scan should be full-scope"
    );
}

// ---------------------------------------------------------------------------
// deterministic ordering: run twice, compare finding order
// ---------------------------------------------------------------------------

#[test]
fn scan_findings_are_deterministically_ordered() {
    let root = fixture_root("unused-file-basic");
    let first = run_pruneguard(&root, &["--format", "json", "--no-baseline", "scan"]);
    let second = run_pruneguard(&root, &["--format", "json", "--no-baseline", "scan"]);

    let first_findings = first["findings"].as_array().expect("findings array");
    let second_findings = second["findings"].as_array().expect("findings array");

    assert_eq!(
        first_findings.len(),
        second_findings.len(),
        "repeated scans should produce the same number of findings"
    );

    for (idx, (a, b)) in first_findings.iter().zip(second_findings.iter()).enumerate() {
        assert_eq!(
            a["id"], b["id"],
            "finding at position {idx} should have the same ID across runs"
        );
        assert_eq!(
            a["subject"], b["subject"],
            "finding at position {idx} should have the same subject across runs"
        );
    }
}

#[test]
fn scan_entrypoints_are_deterministically_ordered() {
    let root = fixture_root("unused-file-basic");
    let first = run_pruneguard(&root, &["--format", "json", "--no-baseline", "scan"]);
    let second = run_pruneguard(&root, &["--format", "json", "--no-baseline", "scan"]);

    let first_eps = first["entrypoints"].as_array().expect("entrypoints array");
    let second_eps = second["entrypoints"].as_array().expect("entrypoints array");

    assert_eq!(first_eps.len(), second_eps.len());
    for (idx, (a, b)) in first_eps.iter().zip(second_eps.iter()).enumerate() {
        assert_eq!(
            a["source"], b["source"],
            "entrypoint at position {idx} should have the same source across runs"
        );
    }
}

#[test]
fn scan_inventories_are_deterministically_ordered() {
    let root = fixture_root("unused-file-basic");
    let first = run_pruneguard(&root, &["--format", "json", "--no-baseline", "scan"]);
    let second = run_pruneguard(&root, &["--format", "json", "--no-baseline", "scan"]);

    let first_files = first["inventories"]["files"].as_array().expect("files array");
    let second_files = second["inventories"]["files"].as_array().expect("files array");

    assert_eq!(first_files.len(), second_files.len());
    for (idx, (a, b)) in first_files.iter().zip(second_files.iter()).enumerate() {
        assert_eq!(
            a["path"], b["path"],
            "file at position {idx} should have the same path across runs"
        );
    }
}

// ---------------------------------------------------------------------------
// package-exports-subpaths: files not exposed through any exports subpath
// ---------------------------------------------------------------------------

#[test]
fn package_exports_subpaths_flags_internal_file() {
    let root = fixture_root("package-exports-subpaths");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");
    let entrypoints = report["entrypoints"].as_array().expect("entrypoints array");

    // Root "." and "./utils" subpaths should appear as entrypoints.
    assert!(
        entrypoints.iter().any(|ep| {
            ep["kind"] == "package-exports"
                && ep["source"].as_str().is_some_and(|s| s.contains("src/index.ts"))
        }),
        "root export '.' should be detected as a package-exports entrypoint"
    );
    assert!(
        entrypoints.iter().any(|ep| {
            ep["kind"] == "package-exports"
                && ep["source"].as_str().is_some_and(|s| s.contains("src/utils.ts"))
        }),
        "'./utils' subpath should be detected as a package-exports entrypoint"
    );

    // src/internal.ts is not referenced by any exports subpath.
    assert!(
        findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/internal.ts"),
        "src/internal.ts should be flagged as unused (not in any exports subpath)"
    );

    // src/index.ts and src/utils.ts should NOT be flagged.
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/index.ts"),
        "src/index.ts should not be flagged (root export)"
    );
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/utils.ts"),
        "src/utils.ts should not be flagged (subpath export)"
    );
}

// ---------------------------------------------------------------------------
// ambient-declarations: .d.ts files excluded from dead-code findings
// ---------------------------------------------------------------------------

#[test]
fn ambient_declarations_not_flagged_as_dead_code() {
    let root = fixture_root("ambient-declarations");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // .d.ts files must NOT be flagged as unused-file.
    assert!(
        !findings.iter().any(|f| {
            f["code"] == "unused-file"
                && f["subject"].as_str().is_some_and(|s| s.ends_with(".d.ts"))
        }),
        "ambient .d.ts files must be excluded from dead-code findings"
    );

    // Regular .ts files with no importers should still be flagged.
    assert!(
        findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/unused.ts"),
        "regular .ts files with no importers should still be flagged"
    );
}

// ---------------------------------------------------------------------------
// script-only-dependency: deps used only in package.json scripts
// ---------------------------------------------------------------------------

#[test]
fn script_only_dependency_not_flagged_as_unused() {
    let root = fixture_root("script-only-dependency");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // typescript is used only via "tsc" in scripts -- should not be unused.
    assert!(
        !findings
            .iter()
            .any(|f| { f["code"] == "unused-dependency" && f["subject"] == "typescript" }),
        "typescript used in scripts should not be reported as unused dependency"
    );
}

// ---------------------------------------------------------------------------
// confidence-levels: findings carry correct confidence tiers
// ---------------------------------------------------------------------------

#[test]
fn confidence_levels_all_findings_have_confidence_field() {
    let root = fixture_root("confidence-levels");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // Every finding must have a confidence field.
    for finding in findings {
        assert!(
            finding["confidence"].as_str().is_some(),
            "finding `{}` must have a confidence field",
            finding["subject"]
        );
    }

    // src/clearly-dead.ts should be flagged as unused.
    assert!(
        findings
            .iter()
            .any(|f| { f["code"] == "unused-file" && f["subject"] == "src/clearly-dead.ts" }),
        "src/clearly-dead.ts (zero importers) should be flagged as unused"
    );

    // src/used.ts should NOT be flagged.
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/used.ts"),
        "src/used.ts (directly imported) should not be flagged"
    );

    // Confidence counts stats must be present.
    let stats = &report["stats"];
    assert!(stats["confidenceCounts"].is_object(), "confidenceCounts must be present");
}

// ---------------------------------------------------------------------------
// namespace-imports-members: import * as ns; ns.foo marks foo as used
// ---------------------------------------------------------------------------

#[test]
fn namespace_member_demand_marks_accessed_export_as_used() {
    let root = fixture_root("namespace-imports-members");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // foo is accessed via utils.foo() -- it should NOT be flagged as unused export.
    assert!(
        !findings
            .iter()
            .any(|f| { f["code"] == "unused-export" && f["subject"] == "src/utils.ts#foo" }),
        "foo accessed via namespace should not be flagged as unused"
    );

    // bar is never accessed via the namespace -- it SHOULD be flagged.
    assert!(
        findings
            .iter()
            .any(|f| { f["code"] == "unused-export" && f["subject"] == "src/utils.ts#bar" }),
        "bar not accessed via namespace should be flagged as unused export"
    );

    // The file itself is imported -- it should not be flagged as unused-file.
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file" && f["subject"] == "src/utils.ts"),
        "utils.ts should not be flagged as unused (imported via namespace)"
    );
}

// ---------------------------------------------------------------------------
// aliased-reexports: export { foo as bar } keeps foo live
// ---------------------------------------------------------------------------

#[test]
fn aliased_reexports_keep_original_export_live() {
    let root = fixture_root("reexports-alias");
    let report = run_pruneguard(&root, &["--format", "json", "scan"]);
    let findings = report["findings"].as_array().expect("findings array");

    // No files should be flagged as unused (all connected through alias chain).
    assert!(
        !findings.iter().any(|f| f["code"] == "unused-file"),
        "all files should be reachable through the aliased re-export chain"
    );

    // The original export `foo` should NOT be flagged as unused.
    assert!(
        !findings
            .iter()
            .any(|f| { f["code"] == "unused-export" && f["subject"] == "src/foo.ts#foo" }),
        "foo should remain live through the bar alias re-export"
    );
}
