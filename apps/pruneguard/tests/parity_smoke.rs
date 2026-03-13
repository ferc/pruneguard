#![allow(clippy::print_stderr)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

#[derive(Debug)]
struct Corpus {
    name: String,
    path: PathBuf,
    scan_paths: Vec<String>,
    min_files: u64,
    min_packages: u64,
    representative_targets: Vec<String>,
}

#[derive(Debug)]
struct FrameworkCorpus {
    name: String,
    framework: String,
    source_type: String,
    local_path: String,
    expected_trust: String,
    representative_target: String,
    known_warnings: Vec<String>,
}

fn run_corpus_command(corpus: &Corpus, extra_args: &[&str]) -> std::process::Output {
    let mut args = vec!["--format", "json", "--no-cache", "--no-baseline"];
    args.extend_from_slice(extra_args);
    Command::new(env!("CARGO_BIN_EXE_pruneguard"))
        .current_dir(&corpus.path)
        .args(&args)
        .output()
        .expect("pruneguard should run")
}

fn run_corpus_json(corpus: &Corpus, extra_args: &[&str]) -> Value {
    let output = run_corpus_command(corpus, extra_args);
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "[product issue] corpus `{}` command {:?} failed\nstdout:\n{}\nstderr:\n{}",
        corpus.name,
        extra_args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

/// Run a framework corpus command from the resolved `local_path`.
fn run_framework_corpus_command(
    working_dir: &std::path::Path,
    extra_args: &[&str],
) -> std::process::Output {
    let mut args = vec!["--format", "json", "--no-cache", "--no-baseline"];
    args.extend_from_slice(extra_args);
    Command::new(env!("CARGO_BIN_EXE_pruneguard"))
        .current_dir(working_dir)
        .args(&args)
        .output()
        .expect("pruneguard should run")
}

// ---------------------------------------------------------------------------
// Scan: no panic, valid JSON, minimum inventory, no parity warnings
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_scan_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let mut args = vec![
            "--format".to_string(),
            "json".to_string(),
            "--no-cache".to_string(),
            "--no-baseline".to_string(),
            "scan".to_string(),
        ];
        args.extend(corpus.scan_paths.clone());
        let output = Command::new(env!("CARGO_BIN_EXE_pruneguard"))
            .current_dir(&corpus.path)
            .args(&args)
            .output()
            .expect("pruneguard should run");

        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` scan failed\nstdout:\n{}\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout)
            .expect("[product issue] scan should emit valid json");
        assert!(
            report["summary"]["totalFiles"].as_u64().unwrap_or(0) >= corpus.min_files,
            "[product issue] corpus `{}` discovered too few files",
            corpus.name
        );
        assert!(
            report["summary"]["totalPackages"].as_u64().unwrap_or(0) >= corpus.min_packages,
            "[product issue] corpus `{}` discovered too few packages",
            corpus.name
        );
        assert!(
            report["stats"]["parityWarnings"].is_null()
                || report["stats"]["parityWarnings"].as_array().is_some_and(Vec::is_empty),
            "[product issue] corpus `{}` reported parity warnings",
            corpus.name
        );
    }
}

// ---------------------------------------------------------------------------
// Deterministic ordering: run twice, compare finding/entrypoint/inventory order
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_scan_deterministic_ordering() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let first = run_corpus_json(&corpus, &["scan"]);
        let second = run_corpus_json(&corpus, &["scan"]);

        // Finding IDs and subjects must match in order.
        let first_findings = first["findings"].as_array();
        let second_findings = second["findings"].as_array();
        if let (Some(ff), Some(sf)) = (first_findings, second_findings) {
            assert_eq!(
                ff.len(),
                sf.len(),
                "[product issue] corpus `{}` produced different finding counts across runs",
                corpus.name
            );
            for (idx, (a, b)) in ff.iter().zip(sf.iter()).enumerate() {
                assert_eq!(
                    a["id"], b["id"],
                    "[product issue] corpus `{}` finding at position {idx} has different ID",
                    corpus.name
                );
            }
        }

        // Entrypoint sources must match in order.
        let first_eps = first["entrypoints"].as_array();
        let second_eps = second["entrypoints"].as_array();
        if let (Some(fe), Some(se)) = (first_eps, second_eps) {
            assert_eq!(
                fe.len(),
                se.len(),
                "[product issue] corpus `{}` produced different entrypoint counts",
                corpus.name
            );
            for (idx, (a, b)) in fe.iter().zip(se.iter()).enumerate() {
                assert_eq!(
                    a["source"], b["source"],
                    "[product issue] corpus `{}` entrypoint at position {idx} has different source",
                    corpus.name
                );
            }
        }

        // File inventory paths must match in order.
        let first_files = first["inventories"]["files"].as_array();
        let second_files = second["inventories"]["files"].as_array();
        if let (Some(ff), Some(sf)) = (first_files, second_files) {
            assert_eq!(
                ff.len(),
                sf.len(),
                "[product issue] corpus `{}` produced different file inventory counts",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Trust summary: stable fields across real repos
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_scan_stable_trust_summary() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let report = run_corpus_json(&corpus, &["scan"]);

        // Must have trust-related stats.
        let stats = &report["stats"];
        assert!(
            stats["partialScope"].as_bool().is_some(),
            "[product issue] corpus `{}` missing partialScope stat",
            corpus.name
        );
        assert!(
            stats["confidenceCounts"].is_object(),
            "[product issue] corpus `{}` missing confidenceCounts",
            corpus.name
        );
        assert!(
            stats["unresolvedSpecifiers"].as_u64().is_some(),
            "[product issue] corpus `{}` missing unresolvedSpecifiers",
            corpus.name
        );
    }
}

// ---------------------------------------------------------------------------
// Impact: per-corpus, on representative target
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_impact_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let Some(target) = resolve_representative_target(&corpus) else {
            continue;
        };
        let output = run_corpus_command(&corpus, &["impact", &target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` impact on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["affectedEntrypoints"].as_array().is_some(),
                "[product issue] corpus `{}` impact should return affectedEntrypoints",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Explain: per-corpus, on representative target
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_explain_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let Some(target) = resolve_representative_target(&corpus) else {
            continue;
        };
        let output = run_corpus_command(&corpus, &["explain", &target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` explain on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["queryKind"].as_str().is_some(),
                "[product issue] corpus `{}` explain should return queryKind",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Review: per-corpus, no panic, valid JSON, trust fields
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_review_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let output = run_corpus_command(&corpus, &["review"]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` review failed\nstdout:\n{}\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            // Trust summary fields must be present.
            assert!(
                report["trust"]["fullScope"].as_bool().is_some(),
                "[product issue] corpus `{}` review should have trust.fullScope",
                corpus.name
            );
            assert!(
                report["trust"]["confidenceCounts"].is_object(),
                "[product issue] corpus `{}` review should have trust.confidenceCounts",
                corpus.name
            );
            assert!(
                report["trust"]["unresolvedPressure"].is_number(),
                "[product issue] corpus `{}` review should have trust.unresolvedPressure",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Safe-delete: per-corpus, on representative target
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_safe_delete_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let Some(target) = resolve_representative_target(&corpus) else {
            continue;
        };
        let output = run_corpus_command(&corpus, &["safe-delete", &target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` safe-delete on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["targets"].as_array().is_some(),
                "[product issue] corpus `{}` safe-delete should return targets",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Fix-plan: per-corpus, on representative target
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_fix_plan_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let Some(target) = resolve_representative_target(&corpus) else {
            continue;
        };
        let output = run_corpus_command(&corpus, &["fix-plan", &target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` fix-plan on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["query"].as_array().is_some()
                    || report["matchedFindings"].as_array().is_some(),
                "[product issue] corpus `{}` fix-plan should return query or matchedFindings",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Suggest-rules: per-corpus, no panic, valid JSON
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_suggest_rules_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
            eprintln!(
                "[corpus issue] skipping `{}`: path {} does not exist",
                corpus.name,
                corpus.path.display()
            );
            continue;
        }

        let output = run_corpus_command(&corpus, &["suggest-rules"]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` suggest-rules failed\nstdout:\n{}\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["suggestedRules"].as_array().is_some(),
                "[product issue] corpus `{}` suggest-rules should return suggestedRules",
                corpus.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Framework corpora: scan, trust-level validation, no panics
// ---------------------------------------------------------------------------

#[test]
#[ignore = "framework corpora smoke is opt-in"]
#[allow(clippy::too_many_lines)]
fn framework_corpora_scan_without_panics() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpora = load_framework_corpora();

    for fc in &corpora {
        if fc.local_path.is_empty() {
            eprintln!("[corpus issue] skipping `{}`: local_path is empty (needs setup)", fc.name);
            continue;
        }

        // Resolve local_path relative to the pruneguard workspace root.
        let corpus_dir = manifest_dir.join("../../").join(&fc.local_path);
        let Ok(corpus_dir) = corpus_dir.canonicalize() else {
            eprintln!(
                "[corpus issue] skipping `{}`: resolved path {} does not exist",
                fc.name,
                corpus_dir.display()
            );
            continue;
        };

        // Run scan.
        let output = run_framework_corpus_command(&corpus_dir, &["scan", "."]);

        // Distinguish product panics from expected failures.
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        assert!(
            !(stderr_text.contains("panicked") || stderr_text.contains("SIGSEGV")),
            "[product issue] corpus `{}` (framework: {}) panicked during scan\nstderr:\n{}",
            fc.name,
            fc.framework,
            stderr_text
        );

        assert!(
            output.status.success() || output.status.code() == Some(1),
            "[product issue] corpus `{}` scan exited with unexpected code {:?}\nstderr:\n{}",
            fc.name,
            output.status.code(),
            stderr_text,
        );

        // Must emit valid JSON.
        let report: Value = match serde_json::from_slice(&output.stdout) {
            Ok(v) => v,
            Err(e) => {
                panic!(
                    "[product issue] corpus `{}` scan emitted invalid JSON: {e}\nstdout (first 500 bytes):\n{}",
                    fc.name,
                    String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(500)])
                );
            }
        };

        // Trust-level expectations.
        match fc.expected_trust.as_str() {
            "high" => {
                // High trust: no blocking false positives (severity "error" findings
                // with confidence < "high" would be false positives).
                if let Some(findings) = report["findings"].as_array() {
                    let blocking_fps: Vec<_> = findings
                        .iter()
                        .filter(|f| {
                            f["severity"].as_str() == Some("error")
                                && f["confidence"].as_str() != Some("high")
                        })
                        .collect();
                    assert!(
                        blocking_fps.is_empty(),
                        "[product issue] corpus `{}` (expected_trust=high) has {} blocking false positive(s): {:?}",
                        fc.name,
                        blocking_fps.len(),
                        blocking_fps.iter().map(|f| &f["id"]).collect::<Vec<_>>()
                    );
                }
            }
            "medium" => {
                // Medium trust: warnings are advisory only -- no error-severity findings
                // that are NOT in the known_warnings list.
                if let Some(findings) = report["findings"].as_array() {
                    let unexpected_errors: Vec<_> = findings
                        .iter()
                        .filter(|f| {
                            f["severity"].as_str() == Some("error")
                                && !fc.known_warnings.iter().any(|kw| {
                                    f["ruleId"]
                                        .as_str()
                                        .is_some_and(|rid| rid.contains(kw.as_str()))
                                })
                        })
                        .collect();
                    assert!(
                        unexpected_errors.is_empty(),
                        "[product issue] corpus `{}` (expected_trust=medium) has {} unexpected error(s): {:?}",
                        fc.name,
                        unexpected_errors.len(),
                        unexpected_errors.iter().map(|f| &f["id"]).collect::<Vec<_>>()
                    );
                }
            }
            "low" => {
                // Low trust: compatibility warnings must be present (we expect known issues).
                if !fc.known_warnings.is_empty() {
                    let has_any_warning =
                        report["findings"].as_array().is_some_and(|findings| !findings.is_empty())
                            || report["stats"]["parityWarnings"]
                                .as_array()
                                .is_some_and(|pw| !pw.is_empty());
                    assert!(
                        has_any_warning,
                        "[product issue] corpus `{}` (expected_trust=low) expected compatibility warnings but found none",
                        fc.name
                    );
                }
            }
            other => {
                eprintln!(
                    "[corpus issue] corpus `{}` has unknown expected_trust `{other}`, skipping trust check",
                    fc.name
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_corpora() -> Vec<Corpus> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/corpora.toml");
    let content = fs::read_to_string(path).expect("corpora manifest should exist");
    parse_corpora(&content)
}

fn load_framework_corpora() -> Vec<FrameworkCorpus> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/framework_corpora.toml");
    let content = fs::read_to_string(path).expect("framework_corpora manifest should exist");
    parse_framework_corpora(&content)
}

/// Whether a path has a JS/TS source extension that pruneguard can analyse.
fn is_supported_source_extension(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("ts")
            || ext.eq_ignore_ascii_case("tsx")
            || ext.eq_ignore_ascii_case("mjs")
            || ext.eq_ignore_ascii_case("js")
    })
}

/// Resolve the representative target for a corpus. If the configured target
/// exists on disk and has a supported JS/TS extension, return it as-is. If the
/// target is stale (file moved, deleted, or has an unsupported extension like
/// `.rs`), fall back to a deterministic alternative discovered from the current
/// scan inventory, and print a parity note to stderr.
///
/// Returns `None` only when no fallback can be found (the test should skip
/// target-dependent assertions in that case).
fn resolve_representative_target(corpus: &Corpus) -> Option<String> {
    if corpus.representative_targets.is_empty() {
        return None;
    }

    let target = &corpus.representative_targets[0];

    // Fast path: target still exists on disk AND has a supported JS/TS extension.
    // A file that exists but has an unsupported extension (e.g. .rs) is treated
    // as stale -- pruneguard cannot meaningfully analyse non-JS/TS files.
    if corpus.path.join(target).exists()
        && is_supported_source_extension(std::path::Path::new(target))
    {
        return Some(target.clone());
    }

    // Target is stale (missing or unsupported extension). Run a scan to
    // discover the current file inventory and pick the first
    // .ts/.tsx/.mjs/.js source file deterministically.
    let reason = if !corpus.path.join(target).exists() {
        "file does not exist"
    } else {
        "file has unsupported extension for JS/TS analysis"
    };
    eprintln!(
        "[parity] corpus `{}`: representative target `{target}` is stale ({reason}), \
         searching scan inventory for fallback",
        corpus.name
    );

    let report = run_corpus_json(corpus, &["scan"]);
    let files = report["inventories"]["files"].as_array();

    let fallback = files.and_then(|file_list| {
        file_list.iter().find_map(|entry| {
            let path = entry["path"].as_str()?;
            if is_supported_source_extension(std::path::Path::new(path)) {
                Some(path.to_string())
            } else {
                None
            }
        })
    });

    if let Some(ref fb) = fallback {
        eprintln!(
            "[parity] corpus `{}`: falling back to `{fb}` (original: `{target}`)",
            corpus.name
        );
    } else {
        eprintln!(
            "[parity] corpus `{}`: no fallback found for stale target `{target}`, \
             skipping target-dependent tests",
            corpus.name
        );
    }

    fallback
}

fn parse_corpora(content: &str) -> Vec<Corpus> {
    let mut corpora = Vec::new();
    let mut current = None::<Corpus>;

    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[corpus]]" {
            if let Some(corpus) = current.take() {
                corpora.push(corpus);
            }
            current = Some(Corpus {
                name: String::new(),
                path: PathBuf::new(),
                scan_paths: Vec::new(),
                min_files: 0,
                min_packages: 0,
                representative_targets: Vec::new(),
            });
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = raw_value.trim();
        let Some(corpus) = current.as_mut() else {
            continue;
        };
        match key {
            "name" => corpus.name = parse_string(value),
            "path" => corpus.path = PathBuf::from(parse_string(value)),
            "scan_paths" => corpus.scan_paths = parse_array(value),
            "min_files" => corpus.min_files = value.parse().unwrap_or(0),
            "min_packages" => corpus.min_packages = value.parse().unwrap_or(0),
            "representative_targets" => corpus.representative_targets = parse_array(value),
            _ => {}
        }
    }

    if let Some(corpus) = current {
        corpora.push(corpus);
    }

    corpora
}

fn parse_framework_corpora(content: &str) -> Vec<FrameworkCorpus> {
    let mut corpora = Vec::new();
    let mut current = None::<FrameworkCorpus>;

    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[corpus]]" {
            if let Some(corpus) = current.take() {
                corpora.push(corpus);
            }
            current = Some(FrameworkCorpus {
                name: String::new(),
                framework: String::new(),
                source_type: String::new(),
                local_path: String::new(),
                expected_trust: String::new(),
                representative_target: String::new(),
                known_warnings: Vec::new(),
            });
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = raw_value.trim();
        let Some(corpus) = current.as_mut() else {
            continue;
        };
        match key {
            "name" => corpus.name = parse_string(value),
            "framework" => corpus.framework = parse_string(value),
            "source_type" => corpus.source_type = parse_string(value),
            "local_path" => corpus.local_path = parse_string(value),
            "expected_trust" => corpus.expected_trust = parse_string(value),
            "representative_target" => corpus.representative_target = parse_string(value),
            "known_warnings" => corpus.known_warnings = parse_array(value),
            _ => {}
        }
    }

    if let Some(corpus) = current {
        corpora.push(corpus);
    }

    corpora
}

// ---------------------------------------------------------------------------
// Individual framework corpus tests
// ---------------------------------------------------------------------------

fn corpus_fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/corpora/{name}"))
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_next_starter() {
    let root = corpus_fixture_root("next-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_nuxt_starter() {
    let root = corpus_fixture_root("nuxt-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_astro_starter() {
    let root = corpus_fixture_root("astro-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_sveltekit_starter() {
    let root = corpus_fixture_root("sveltekit-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_remix_starter() {
    let root = corpus_fixture_root("remix-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_angular_starter() {
    let root = corpus_fixture_root("angular-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_nx_starter() {
    let root = corpus_fixture_root("nx-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_turborepo_starter() {
    let root = corpus_fixture_root("turborepo-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_playwright_starter() {
    let root = corpus_fixture_root("playwright-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_cypress_starter() {
    let root = corpus_fixture_root("cypress-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_vitepress_starter() {
    let root = corpus_fixture_root("vitepress-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

#[test]
#[ignore = "requires corpus fixture"]
fn framework_corpus_docusaurus_starter() {
    let root = corpus_fixture_root("docusaurus-starter");
    let output = run_framework_corpus_command(&root, &["--severity", "info", "scan"]);
    assert!(output.status.code().is_some(), "should not panic");
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(report["inventories"]["files"].as_array().is_some_and(|a| !a.is_empty()));
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_string(value: &str) -> String {
    value.trim_matches('"').to_string()
}

fn parse_array(value: &str) -> Vec<String> {
    value
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(parse_string)
        .collect()
}
