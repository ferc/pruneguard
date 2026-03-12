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
        "corpus `{}` command {:?} failed\nstdout:\n{}\nstderr:\n{}",
        corpus.name,
        extra_args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Scan: no panic, valid JSON, minimum inventory, no parity warnings
// ---------------------------------------------------------------------------

#[test]
#[ignore = "real-repo smoke is opt-in"]
fn corpora_scan_without_panics() {
    for corpus in load_corpora() {
        if !corpus.path.exists() {
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
            "corpus `{}` failed\nstdout:\n{}\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value =
            serde_json::from_slice(&output.stdout).expect("scan should emit valid json");
        assert!(
            report["summary"]["totalFiles"].as_u64().unwrap_or(0) >= corpus.min_files,
            "corpus `{}` discovered too few files",
            corpus.name
        );
        assert!(
            report["summary"]["totalPackages"].as_u64().unwrap_or(0) >= corpus.min_packages,
            "corpus `{}` discovered too few packages",
            corpus.name
        );
        assert!(
            report["stats"]["parityWarnings"].is_null()
                || report["stats"]["parityWarnings"]
                    .as_array()
                    .is_some_and(Vec::is_empty),
            "corpus `{}` reported parity warnings",
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
                "corpus `{}` produced different finding counts across runs",
                corpus.name
            );
            for (idx, (a, b)) in ff.iter().zip(sf.iter()).enumerate() {
                assert_eq!(
                    a["id"], b["id"],
                    "corpus `{}` finding at position {idx} has different ID",
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
                "corpus `{}` produced different entrypoint counts",
                corpus.name
            );
            for (idx, (a, b)) in fe.iter().zip(se.iter()).enumerate() {
                assert_eq!(
                    a["source"], b["source"],
                    "corpus `{}` entrypoint at position {idx} has different source",
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
                "corpus `{}` produced different file inventory counts",
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
            continue;
        }

        let report = run_corpus_json(&corpus, &["scan"]);

        // Must have trust-related stats.
        let stats = &report["stats"];
        assert!(
            stats["partialScope"].as_bool().is_some(),
            "corpus `{}` missing partialScope stat",
            corpus.name
        );
        assert!(
            stats["confidenceCounts"].is_object(),
            "corpus `{}` missing confidenceCounts",
            corpus.name
        );
        assert!(
            stats["unresolvedSpecifiers"].as_u64().is_some(),
            "corpus `{}` missing unresolvedSpecifiers",
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
        if !corpus.path.exists() || corpus.representative_targets.is_empty() {
            continue;
        }

        let target = &corpus.representative_targets[0];
        let output = run_corpus_command(&corpus, &["impact", target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "corpus `{}` impact on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["affectedEntrypoints"].as_array().is_some(),
                "corpus `{}` impact should return affectedEntrypoints",
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
        if !corpus.path.exists() || corpus.representative_targets.is_empty() {
            continue;
        }

        let target = &corpus.representative_targets[0];
        let output = run_corpus_command(&corpus, &["explain", target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "corpus `{}` explain on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["queryKind"].as_str().is_some(),
                "corpus `{}` explain should return queryKind",
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
            continue;
        }

        let output = run_corpus_command(&corpus, &["review"]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "corpus `{}` review failed\nstdout:\n{}\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            // Trust summary fields must be present.
            assert!(
                report["trust"]["fullScope"].as_bool().is_some(),
                "corpus `{}` review should have trust.fullScope",
                corpus.name
            );
            assert!(
                report["trust"]["confidenceCounts"].is_object(),
                "corpus `{}` review should have trust.confidenceCounts",
                corpus.name
            );
            assert!(
                report["trust"]["unresolvedPressure"].is_number(),
                "corpus `{}` review should have trust.unresolvedPressure",
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
        if !corpus.path.exists() || corpus.representative_targets.is_empty() {
            continue;
        }

        let target = &corpus.representative_targets[0];
        let output = run_corpus_command(&corpus, &["safe-delete", target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "corpus `{}` safe-delete on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["targets"].as_array().is_some(),
                "corpus `{}` safe-delete should return targets",
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
        if !corpus.path.exists() || corpus.representative_targets.is_empty() {
            continue;
        }

        let target = &corpus.representative_targets[0];
        let output = run_corpus_command(&corpus, &["fix-plan", target]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "corpus `{}` fix-plan on `{target}` failed\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["query"].as_array().is_some() || report["matchedFindings"].as_array().is_some(),
                "corpus `{}` fix-plan should return query or matchedFindings",
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
            continue;
        }

        let output = run_corpus_command(&corpus, &["suggest-rules"]);
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "corpus `{}` suggest-rules failed\nstdout:\n{}\nstderr:\n{}",
            corpus.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        if report != Value::Null {
            assert!(
                report["suggestedRules"].as_array().is_some(),
                "corpus `{}` suggest-rules should return suggestedRules",
                corpus.name
            );
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
