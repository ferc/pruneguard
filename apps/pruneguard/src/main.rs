#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_report::{ConfidenceCounts, Finding, FindingConfidence, FindingSeverity};

mod cli;
mod migrate;

fn main() -> ExitCode {
    let options = cli::options().run();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    match run(options) {
        Ok(exit) => exit,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::from(2)
        }
    }
}

fn run(options: cli::Options) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");
    let profile = to_entrypoint_profile(options.global.profile);

    match options.command {
        cli::Command::Scan { paths } => {
            let config_cwd = paths.first().map_or_else(
                || cwd.clone(),
                |path| {
                    if path.is_absolute() { path.clone() } else { cwd.join(path) }
                },
            );
            let config = load_config_or_default(&config_cwd, options.config.as_deref())?;
            let scan = pruneguard::scan_with_options(
                &cwd,
                &config,
                &paths,
                profile,
                &pruneguard::ScanOptions {
                    config_dir: Some(config_cwd),
                    changed_since: options.global.changed_since.clone(),
                    focus: options.global.focus.clone(),
                    no_cache: options.global.no_cache,
                    no_baseline: options.global.no_baseline,
                    require_full_scope: options.global.require_full_scope,
                },
            )?;
            handle_scan_report(scan, &options.global)
        }
        cli::Command::Impact { target } => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            if matches!(options.global.format, cli::OutputFormat::Dot) {
                miette::bail!("dot output is only supported for scan in this phase");
            }
            let report = pruneguard::impact_with_options(
                &cwd,
                &config,
                &target,
                profile,
                &pruneguard::ImpactOptions { focus: options.global.focus.clone() },
            )?;
            if matches!(options.global.format, cli::OutputFormat::Text) {
                print_impact_text(&report, options.global.focus.as_deref());
            } else {
                print_report(&report, options.global.format)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Explain { query } => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            if matches!(options.global.format, cli::OutputFormat::Dot) {
                miette::bail!("dot output is only supported for scan in this phase");
            }
            let report = pruneguard::explain_with_options(
                &cwd,
                &config,
                &query,
                profile,
                &pruneguard::ExplainOptions { focus: options.global.focus.clone() },
            )?;
            if matches!(options.global.format, cli::OutputFormat::Text) {
                print_explain_text(&report, options.global.focus.as_deref());
            } else {
                print_report(&report, options.global.format)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Init => {
            pruneguard_config::PruneguardConfig::init()?;
            eprintln!("Created pruneguard.json");
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::PrintConfig => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            let json = serde_json::to_string_pretty(&config).expect("failed to serialize config");
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Debug(debug_cmd) => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            run_debug(debug_cmd, &config, profile)
        }
        cli::Command::Migrate(ref migrate_cmd) => run_migrate(migrate_cmd, options.global.format),
    }
}

fn run_debug(
    cmd: cli::DebugCommand,
    config: &pruneguard_config::PruneguardConfig,
    profile: EntrypointProfile,
) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");

    match cmd {
        cli::DebugCommand::Resolve { specifier, from } => {
            let result =
                pruneguard_resolver::debug_resolve(&cwd, &config.resolver, &specifier, &from);
            println!("{result}");
            Ok(ExitCode::SUCCESS)
        }
        cli::DebugCommand::Entrypoints => {
            let entrypoints = pruneguard::debug_entrypoints(&cwd, config, profile)?;
            for entrypoint in &entrypoints {
                println!("{entrypoint}");
            }
            Ok(ExitCode::SUCCESS)
        }
        cli::DebugCommand::Runtime => {
            let binary = std::env::current_exe()
                .map_or_else(|_| "unknown".to_string(), |p| p.display().to_string());
            let schema_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|dir| dir.join("configuration_schema.json")))
                .map_or_else(|| "unknown".to_string(), |p| p.display().to_string());
            println!("binary: {binary}");
            println!("platform: {}-{}", std::env::consts::OS, std::env::consts::ARCH);
            println!("version: {}", env!("CARGO_PKG_VERSION"));
            println!("cwd: {}", cwd.display());
            println!("schema_path: {schema_path}");
            println!("resolution_source: binary");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_migrate(cmd: &cli::MigrateCommand, format: cli::OutputFormat) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");
    if matches!(format, cli::OutputFormat::Sarif | cli::OutputFormat::Dot) {
        miette::bail!("sarif and dot output are not supported for migration commands");
    }

    match cmd {
        cli::MigrateCommand::Knip { file } => {
            let output = migrate::migrate_knip(&cwd, file.as_deref())?;
            print_migration_output(&output, format)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::MigrateCommand::Depcruise { file, node } => {
            let output = migrate::migrate_depcruise(&cwd, file.as_deref(), *node)?;
            print_migration_output(&output, format)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn load_config_or_default(
    cwd: &std::path::Path,
    config_path: Option<&std::path::Path>,
) -> miette::Result<pruneguard_config::PruneguardConfig> {
    match pruneguard_config::PruneguardConfig::load(cwd, config_path) {
        Ok(config) => Ok(config),
        Err(pruneguard_config::ConfigError::NotFound) => {
            Ok(pruneguard_config::PruneguardConfig::default())
        }
        Err(err) => Err(err.into()),
    }
}

fn handle_scan_report(
    mut scan: pruneguard::ScanExecution,
    flags: &cli::GlobalFlags,
) -> miette::Result<ExitCode> {
    let report = &mut scan.report;
    report.findings = filtered_findings(&report.findings, flags.severity, flags.max_findings);
    let (errors, warnings, infos) = summarize_findings(&report.findings);
    report.summary.total_findings = report.findings.len();
    report.summary.errors = errors;
    report.summary.warnings = warnings;
    report.summary.infos = infos;
    report.stats.confidence_counts =
        report.findings.iter().fold(ConfidenceCounts::default(), |mut counts, finding| {
            match finding.confidence {
                FindingConfidence::High => counts.high += 1,
                FindingConfidence::Medium => counts.medium += 1,
                FindingConfidence::Low => counts.low += 1,
            }
            counts
        });

    if matches!(flags.format, cli::OutputFormat::Dot) {
        println!("{}", pruneguard::render_module_graph_dot(&scan.build, &report.findings));
    } else {
        if matches!(flags.format, cli::OutputFormat::Text) && flags.no_baseline {
            println!("baseline: disabled by --no-baseline");
        }
        print_report(&report, flags.format)?;
    }

    let exit = if report.findings.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) };
    Ok(exit)
}

fn filtered_findings(
    findings: &[Finding],
    threshold: cli::Severity,
    max_findings: Option<usize>,
) -> Vec<Finding> {
    let mut findings = findings
        .iter()
        .filter(|finding| severity_at_or_above(finding.severity, threshold))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(limit) = max_findings {
        findings.truncate(limit);
    }
    findings
}

const fn severity_at_or_above(severity: FindingSeverity, threshold: cli::Severity) -> bool {
    match threshold {
        cli::Severity::Error => matches!(severity, FindingSeverity::Error),
        cli::Severity::Warn => {
            matches!(severity, FindingSeverity::Error | FindingSeverity::Warn)
        }
        cli::Severity::Info => true,
    }
}

fn summarize_findings(findings: &[Finding]) -> (usize, usize, usize) {
    findings.iter().fold((0, 0, 0), |(errors, warnings, infos), finding| match finding.severity {
        FindingSeverity::Error => (errors + 1, warnings, infos),
        FindingSeverity::Warn => (errors, warnings + 1, infos),
        FindingSeverity::Info => (errors, warnings, infos + 1),
    })
}

const fn to_entrypoint_profile(profile: cli::Profile) -> EntrypointProfile {
    match profile {
        cli::Profile::Production => EntrypointProfile::Production,
        cli::Profile::Development => EntrypointProfile::Development,
        cli::Profile::All => EntrypointProfile::Both,
    }
}

fn print_report<T: serde::Serialize>(report: &T, format: cli::OutputFormat) -> miette::Result<()> {
    match format {
        cli::OutputFormat::Text => print_text_report(report)?,
        cli::OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(report).expect("serialize report"));
        }
        cli::OutputFormat::Sarif => {
            println!("{}", render_sarif(report)?);
        }
        cli::OutputFormat::Dot => {
            miette::bail!("dot output requires a scan execution");
        }
    }
    Ok(())
}

fn print_migration_output(
    output: &migrate::MigrationOutput,
    format: cli::OutputFormat,
) -> miette::Result<()> {
    match format {
        cli::OutputFormat::Text => {
            println!(
                "{}",
                serde_json::to_string_pretty(&output.config).expect("serialize migration config")
            );
            for warning in &output.warnings {
                eprintln!("warning: {warning}");
            }
        }
        cli::OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(output).expect("serialize migration output")
            );
        }
        cli::OutputFormat::Sarif | cli::OutputFormat::Dot => {
            miette::bail!("sarif and dot output are not supported for migration commands");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn print_text_report<T: serde::Serialize>(report: &T) -> miette::Result<()> {
    let value = serde_json::to_value(report).map_err(|err| miette::miette!("{err}"))?;
    match value {
        serde_json::Value::Object(map) if map.contains_key("summary") => {
            let summary = map.get("summary").and_then(serde_json::Value::as_object);
            let findings = map
                .get("findings")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let entrypoints =
                map.get("entrypoints").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
            let total_files = summary
                .and_then(|summary| summary.get("totalFiles"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let total_packages = summary
                .and_then(|summary| summary.get("totalPackages"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            println!("repo summary");
            println!("files: {total_files}");
            println!("packages: {total_packages}");
            println!("entrypoints: {entrypoints}");
            println!("findings: {}", findings.len());
            if let Some(stats) = map.get("stats").and_then(serde_json::Value::as_object) {
                let unresolved = stats
                    .get("unresolvedSpecifiers")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let resolved =
                    stats.get("filesResolved").and_then(serde_json::Value::as_u64).unwrap_or(0);
                let duration =
                    stats.get("durationMs").and_then(serde_json::Value::as_u64).unwrap_or(0);
                println!("duration ms: {duration}");

                // Trust summary
                println!();
                println!("trust summary");
                let partial_scope =
                    stats.get("partialScope").and_then(serde_json::Value::as_bool).unwrap_or(false);
                let full_scope_required = stats
                    .get("fullScopeRequired")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                println!("scope: {}", if partial_scope { "partial (advisory)" } else { "full" });
                if partial_scope {
                    let reason = stats
                        .get("partialScopeReason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("scan paths narrowed analysis to a partial scope.");
                    println!("  {reason}");
                    if full_scope_required {
                        println!("  full-scope enforcement was requested for this run.");
                    }
                }

                let baseline_applied = stats
                    .get("baselineApplied")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                println!(
                    "baseline: {}",
                    if baseline_applied { "applied" } else { "disabled or not found" }
                );
                let baseline_profile_mismatch = stats
                    .get("baselineProfileMismatch")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if baseline_profile_mismatch {
                    println!("  baseline profile differs from the current profile.");
                }

                println!("unresolved specifiers: {unresolved}");
                if resolved > 0 && unresolved > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let pressure_pct = (unresolved as f64 / (resolved + unresolved) as f64) * 100.0;
                    if pressure_pct > 5.0 {
                        println!(
                            "  unresolved pressure: {pressure_pct:.1}% — findings may have lower accuracy"
                        );
                    }
                }

                if let Some(confidence) =
                    stats.get("confidenceCounts").and_then(serde_json::Value::as_object)
                {
                    println!(
                        "confidence: high={}, medium={}, low={}",
                        confidence.get("high").and_then(serde_json::Value::as_u64).unwrap_or(0),
                        confidence.get("medium").and_then(serde_json::Value::as_u64).unwrap_or(0),
                        confidence.get("low").and_then(serde_json::Value::as_u64).unwrap_or(0),
                    );
                }

                let focus_applied =
                    stats.get("focusApplied").and_then(serde_json::Value::as_bool).unwrap_or(false);
                if focus_applied {
                    let focused_files =
                        stats.get("focusedFiles").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    let focused_findings = stats
                        .get("focusedFindings")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    println!();
                    println!("focus summary");
                    println!("focused files: {focused_files}");
                    println!("focused findings: {focused_findings}");
                    println!("findings were filtered after full analysis.");
                }
            }
            if !findings.is_empty() {
                println!();
                for finding in findings {
                    let severity = finding
                        .get("severity")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("warn");
                    let code = finding
                        .get("code")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("finding");
                    let confidence = finding
                        .get("confidence")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("medium");
                    let subject =
                        finding.get("subject").and_then(serde_json::Value::as_str).unwrap_or("");
                    let message =
                        finding.get("message").and_then(serde_json::Value::as_str).unwrap_or("");
                    println!("[{severity}] {code} {subject} ({confidence})");
                    println!("  {message}");
                    if let Some(evidence) =
                        finding.get("evidence").and_then(serde_json::Value::as_array)
                    {
                        for item in evidence.iter().take(3) {
                            let description = item
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("");
                            if !description.is_empty() {
                                println!("  proof: {description}");
                            }
                        }
                    }
                }
            }
        }
        _ => println!("{}", serde_json::to_string_pretty(report).expect("serialize report")),
    }
    Ok(())
}

fn print_impact_text(report: &pruneguard_report::ImpactReport, focus: Option<&str>) {
    println!("impact target: {}", report.target);
    println!("affected entrypoints: {}", report.affected_entrypoints.len());
    println!("affected packages: {}", report.affected_packages.len());
    println!("affected files: {}", report.affected_files.len());
    if let Some(focus) = focus {
        println!("focus: {focus}");
        println!(
            "{}",
            if report.focus_filtered {
                "returned items were filtered after full-graph impact analysis."
            } else {
                "focus did not remove any affected nodes."
            }
        );
    }
    if !report.affected_files.is_empty() {
        println!();
        for file in &report.affected_files {
            println!("file: {file}");
        }
    }
}

fn print_explain_text(report: &pruneguard_report::ExplainReport, focus: Option<&str>) {
    let match_kind = match report.query_kind {
        pruneguard_report::ExplainQueryKind::Finding => "finding",
        pruneguard_report::ExplainQueryKind::File => "file",
        pruneguard_report::ExplainQueryKind::Export => "export",
    };
    println!("query: {} ({match_kind})", report.query);
    if let Some(node) = &report.matched_node {
        println!("matched node: {node}");
    } else {
        println!("matched node: none");
    }
    if let Some(focus) = focus {
        println!(
            "focus: {focus} ({})",
            if report.focus_filtered {
                "related findings or proofs were filtered"
            } else {
                "related findings and proofs are within focus"
            }
        );
    }
    println!("proofs: {}", report.proofs.len());
    println!("related findings: {}", report.related_findings.len());
}

fn render_sarif<T: serde::Serialize>(report: &T) -> miette::Result<String> {
    let value = serde_json::to_value(report).map_err(|err| miette::miette!("{err}"))?;
    let findings =
        value.get("findings").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();

    let results = findings
        .into_iter()
        .map(|finding| {
            let severity = finding
                .get("severity")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("warning");
            let level = match severity {
                "error" => "error",
                "info" => "note",
                _ => "warning",
            };
            serde_json::json!({
                "ruleId": finding.get("code").and_then(serde_json::Value::as_str).unwrap_or("pruneguard"),
                "level": level,
                "message": {
                    "text": finding.get("message").and_then(serde_json::Value::as_str).unwrap_or("")
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": finding.get("subject").and_then(serde_json::Value::as_str).unwrap_or("")
                        }
                    }
                }]
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "pruneguard"
                }
            },
            "results": results
        }]
    }))
    .map_err(|err| miette::miette!("{err}"))
}
