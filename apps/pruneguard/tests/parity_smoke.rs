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
}

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
                    .is_some_and(|warnings| warnings.is_empty()),
            "corpus `{}` reported parity warnings",
            corpus.name
        );
    }
}

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
