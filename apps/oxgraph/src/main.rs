#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use oxgraph_entrypoints::EntrypointProfile;
use oxgraph_report::{Finding, FindingSeverity};

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
            let config_cwd = paths.first().map_or_else(|| cwd.clone(), |path| {
                if path.is_absolute() { path.clone() } else { cwd.join(path) }
            });
            let config = load_config_or_default(&config_cwd, options.config.as_deref())?;
            let scan = oxgraph::scan_with_options(
                &cwd,
                &config,
                &paths,
                profile,
                &oxgraph::ScanOptions {
                    config_dir: Some(config_cwd),
                    changed_since: options.global.changed_since.clone(),
                    no_cache: options.global.no_cache,
                },
            )?;
            handle_scan_report(scan, &options.global)
        }
        cli::Command::Impact { target } => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            if matches!(options.global.format, cli::OutputFormat::Dot) {
                miette::bail!("dot output is only supported for scan in this phase");
            }
            let report =
                oxgraph::impact(&cwd, &config, &target, profile)?;
            print_report(&report, options.global.format)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Explain { query } => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            if matches!(options.global.format, cli::OutputFormat::Dot) {
                miette::bail!("dot output is only supported for scan in this phase");
            }
            let report =
                oxgraph::explain(&cwd, &config, &query, profile)?;
            print_report(&report, options.global.format)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Init => {
            oxgraph_config::OxgraphConfig::init()?;
            eprintln!("Created oxgraph.json");
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::PrintConfig => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            let json = serde_json::to_string_pretty(&config)
                .expect("failed to serialize config");
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
    config: &oxgraph_config::OxgraphConfig,
    profile: EntrypointProfile,
) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");

    match cmd {
        cli::DebugCommand::Resolve { specifier, from } => {
            let result = oxgraph_resolver::debug_resolve(&cwd, &config.resolver, &specifier, &from);
            println!("{result}");
            Ok(ExitCode::SUCCESS)
        }
        cli::DebugCommand::Entrypoints => {
            let entrypoints = oxgraph::debug_entrypoints(&cwd, config, profile)?;
            for entrypoint in &entrypoints {
                println!("{entrypoint}");
            }
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
) -> miette::Result<oxgraph_config::OxgraphConfig> {
    match oxgraph_config::OxgraphConfig::load(cwd, config_path) {
        Ok(config) => Ok(config),
        Err(oxgraph_config::ConfigError::NotFound) => Ok(oxgraph_config::OxgraphConfig::default()),
        Err(err) => Err(err.into()),
    }
}

fn handle_scan_report(
    mut scan: oxgraph::ScanExecution,
    flags: &cli::GlobalFlags,
) -> miette::Result<ExitCode> {
    let report = &mut scan.report;
    report.findings = filtered_findings(&report.findings, flags.severity, flags.max_findings);
    let (errors, warnings, infos) = summarize_findings(&report.findings);
    report.summary.total_findings = report.findings.len();
    report.summary.errors = errors;
    report.summary.warnings = warnings;
    report.summary.infos = infos;

    if matches!(flags.format, cli::OutputFormat::Dot) {
        println!("{}", oxgraph::render_module_graph_dot(&scan.build, &report.findings));
    } else {
        print_report(&report, flags.format)?;
    }

    let exit = if report.findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    };
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
    findings.iter().fold((0, 0, 0), |(errors, warnings, infos), finding| {
        match finding.severity {
            FindingSeverity::Error => (errors + 1, warnings, infos),
            FindingSeverity::Warn => (errors, warnings + 1, infos),
            FindingSeverity::Info => (errors, warnings, infos + 1),
        }
    })
}

const fn to_entrypoint_profile(profile: cli::Profile) -> EntrypointProfile {
    match profile {
        cli::Profile::Production => EntrypointProfile::Production,
        cli::Profile::Development => EntrypointProfile::Development,
        cli::Profile::All => EntrypointProfile::Both,
    }
}

fn print_report<T: serde::Serialize>(
    report: &T,
    format: cli::OutputFormat,
) -> miette::Result<()> {
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
            let entrypoints = map
                .get("entrypoints")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let total_files = summary
                .and_then(|summary| summary.get("totalFiles"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let total_packages = summary
                .and_then(|summary| summary.get("totalPackages"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            println!("files: {total_files}");
            println!("packages: {total_packages}");
            println!("entrypoints: {entrypoints}");
            println!("findings: {}", findings.len());
            if let Some(stats) = map.get("stats").and_then(serde_json::Value::as_object) {
                let unresolved = stats
                    .get("unresolvedSpecifiers")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let duration = stats
                    .get("durationMs")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                println!("unresolved specifiers: {unresolved}");
                println!("duration ms: {duration}");
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
                    let subject = finding
                        .get("subject")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let message = finding
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    println!("[{severity}] {code} {subject}");
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

fn render_sarif<T: serde::Serialize>(report: &T) -> miette::Result<String> {
    let value = serde_json::to_value(report).map_err(|err| miette::miette!("{err}"))?;
    let findings = value
        .get("findings")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

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
                "ruleId": finding.get("code").and_then(serde_json::Value::as_str).unwrap_or("oxgraph"),
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
                    "name": "oxgraph"
                }
            },
            "results": results
        }]
    }))
    .map_err(|err| miette::miette!("{err}"))
}
