#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use pruneguard_entrypoints::EntrypointProfile;
use pruneguard_report::{
    ConfidenceCounts, Finding, FindingConfidence, FindingSeverity, ReviewReport,
};

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

#[allow(clippy::too_many_lines)]
fn run(options: cli::Options) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");
    let profile = to_entrypoint_profile(options.global.profile);

    // Resolve the effective daemon mode: `auto` in CI becomes `off`.
    let effective_daemon = resolve_daemon_mode(options.global.daemon);

    match options.command {
        cli::Command::Scan { paths } => {
            // Try daemon-backed scan first.
            if let Some(exit) = try_daemon_scan(&cwd, effective_daemon, &paths, &options.global)? {
                return Ok(exit);
            }

            // Fall back to one-shot scan.
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
            // Try daemon-backed impact first.
            if let Some(exit) = try_daemon_impact(&cwd, effective_daemon, &target, &options.global)?
            {
                return Ok(exit);
            }

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
            // Try daemon-backed explain first.
            if let Some(exit) = try_daemon_explain(&cwd, effective_daemon, &query, &options.global)?
            {
                return Ok(exit);
            }

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
        cli::Command::BarePathsError { paths } => {
            eprintln!("error: unexpected positional arguments: {}", paths.join(", "));
            eprintln!();
            eprintln!("  Use: pruneguard scan <paths...>   for partial-scope analysis");
            eprintln!("  Use: pruneguard                   to review your repo");
            eprintln!();
            eprintln!("Run pruneguard --help for all commands.");
            Ok(ExitCode::from(2))
        }
        cli::Command::Review { strict_trust } => {
            // Try daemon-backed review first.
            if let Some(exit) = try_daemon_review(&cwd, effective_daemon, &options.global)? {
                return Ok(exit);
            }

            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            let report = pruneguard::review(
                &cwd,
                &config,
                profile,
                &pruneguard::ReviewOptions {
                    config_dir: Some(cwd.clone()),
                    base_ref: options.global.changed_since.clone(),
                    no_cache: options.global.no_cache,
                    no_baseline: options.global.no_baseline,
                    strict_trust,
                },
            )?;
            if matches!(options.global.format, cli::OutputFormat::Text) {
                print_review_text(&report);
            } else {
                print_report(&report, options.global.format)?;
            }
            let exit = if report.blocking_findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            };
            Ok(exit)
        }
        cli::Command::SafeDelete { targets } => {
            // Try daemon-backed safe-delete first.
            if let Some(exit) =
                try_daemon_safe_delete(&cwd, effective_daemon, &targets, &options.global)?
            {
                return Ok(exit);
            }

            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            if targets.is_empty() {
                miette::bail!("safe-delete requires at least one target");
            }
            let report = pruneguard::safe_delete(
                &cwd,
                &config,
                &targets,
                profile,
                &pruneguard::SafeDeleteOptions {
                    config_dir: Some(cwd.clone()),
                    no_cache: options.global.no_cache,
                },
            )?;
            print_report(&report, options.global.format)?;
            let exit = if report.blocked.is_empty() && report.needs_review.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            };
            Ok(exit)
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
        cli::Command::FixPlan { targets } => {
            // Try daemon-backed fix-plan first.
            if let Some(exit) =
                try_daemon_fix_plan(&cwd, effective_daemon, &targets, &options.global)?
            {
                return Ok(exit);
            }

            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            if targets.is_empty() {
                miette::bail!("fix-plan requires at least one target");
            }
            let report = pruneguard::fix_plan(
                &cwd,
                &config,
                &targets,
                profile,
                &pruneguard::FixPlanOptions {
                    config_dir: Some(cwd.clone()),
                    no_cache: options.global.no_cache,
                },
            )?;
            print_report(&report, options.global.format)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::SuggestRules => {
            // Try daemon-backed suggest-rules first.
            if let Some(exit) = try_daemon_suggest_rules(&cwd, effective_daemon, &options.global)? {
                return Ok(exit);
            }

            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            let report = pruneguard::suggest_rules(
                &cwd,
                &config,
                profile,
                &pruneguard::SuggestRulesOptions {
                    config_dir: Some(cwd.clone()),
                    no_cache: options.global.no_cache,
                },
            )?;
            print_report(&report, options.global.format)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::CompatibilityReport => {
            // Try daemon-backed compatibility-report first.
            if let Some(exit) =
                try_daemon_compatibility_report(&cwd, effective_daemon, &options.global)?
            {
                return Ok(exit);
            }

            let _config = load_config_or_default(&cwd, options.config.as_deref())?;
            let report = pruneguard::compatibility_report(&cwd, profile)?;
            if matches!(options.global.format, cli::OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&report).expect("serialize report"));
            } else {
                println!("--- compatibility report ---");
                println!();
                if !report.supported_frameworks.is_empty() {
                    println!("supported frameworks:");
                    for fw in &report.supported_frameworks {
                        println!("  {fw}");
                    }
                }
                if !report.heuristic_frameworks.is_empty() {
                    println!("heuristic frameworks (partial support):");
                    for fw in &report.heuristic_frameworks {
                        println!("  {fw}");
                    }
                }
                if !report.unsupported_signals.is_empty() {
                    println!();
                    println!("unsupported signals:");
                    for sig in &report.unsupported_signals {
                        print!("  {} (source: {})", sig.signal, sig.source);
                        if let Some(suggestion) = &sig.suggestion {
                            print!(" -- {suggestion}");
                        }
                        println!();
                    }
                }
                if !report.warnings.is_empty() {
                    println!();
                    println!("warnings:");
                    for warning in &report.warnings {
                        println!(
                            "  [{}] {} (severity: {})",
                            warning.code, warning.message, warning.severity
                        );
                    }
                }
                if !report.trust_downgrades.is_empty() {
                    println!();
                    println!("trust downgrades:");
                    for td in &report.trust_downgrades {
                        println!(
                            "  {} (scope: {}, severity: {})",
                            td.reason, td.scope, td.severity
                        );
                    }
                }
                println!();
                println!("----------------------------");
            }
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Debug(debug_cmd) => {
            let config = load_config_or_default(&cwd, options.config.as_deref())?;
            run_debug(debug_cmd, &config, profile, options.global.format)
        }
        cli::Command::Migrate(ref migrate_cmd) => run_migrate(migrate_cmd, options.global.format),
        cli::Command::Daemon(daemon_cmd) => run_daemon(&daemon_cmd),
    }
}

/// Resolve the effective daemon mode: `Auto` becomes `Off` in CI.
fn resolve_daemon_mode(mode: cli::DaemonMode) -> cli::DaemonMode {
    match mode {
        cli::DaemonMode::Auto if pruneguard_daemon::client::is_ci() => {
            tracing::debug!("CI detected; daemon auto-start disabled");
            cli::DaemonMode::Off
        }
        other => other,
    }
}

/// Attempt to connect to a daemon (auto-starting if needed) and return a client.
///
/// Returns `Ok(None)` if the daemon mode is `Off` or the daemon is not available
/// and auto-start is not applicable.
///
/// For `Auto` mode, if no daemon is currently running, a daemon process is
/// spawned in the background for future runs and `None` is returned so the
/// current invocation falls back to one-shot. This avoids making the first
/// run slow due to daemon startup.
fn try_daemon_client(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
) -> miette::Result<Option<pruneguard_daemon::DaemonClient>> {
    let project_root = find_project_root_dir(cwd);
    match mode {
        cli::DaemonMode::Off => Ok(None),
        cli::DaemonMode::Auto => {
            match pruneguard_daemon::DaemonClient::try_connect_or_background_start(&project_root) {
                Ok(Some(client)) => Ok(Some(client)),
                Ok(None) => {
                    tracing::debug!(
                        "no running daemon; spawned one in background, falling back to one-shot"
                    );
                    Ok(None)
                }
                Err(err) => {
                    // In auto mode, silently fall back to one-shot on any error.
                    tracing::debug!("daemon auto-connect failed, falling back to one-shot: {err}");
                    Ok(None)
                }
            }
        }
        cli::DaemonMode::Required => {
            match pruneguard_daemon::DaemonClient::connect_or_start(&project_root) {
                Ok(client) => Ok(Some(client)),
                Err(err) => {
                    miette::bail!("daemon required but not available: {err}");
                }
            }
        }
    }
}

/// Try a daemon-backed scan. Returns `Some(exit)` if the daemon handled it,
/// or `None` if the caller should fall back to one-shot.
fn try_daemon_scan(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    paths: &[std::path::PathBuf],
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::Scan {
        paths: paths.iter().map(|p| p.to_string_lossy().to_string()).collect(),
        changed_since: flags.changed_since.clone(),
        focus: flags.focus.clone(),
    };

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::ScanResult { report }) => {
            eprintln!("(daemon-backed scan)");
            print_daemon_report(&report, flags.format);
            Ok(Some(ExitCode::SUCCESS))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            tracing::debug!("daemon scan returned error: {message}");
            // Fall back to one-shot on daemon error in auto mode.
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon scan failed: {message}");
            }
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon scan failed: {err}");
            }
            tracing::debug!("daemon scan request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed review.
fn try_daemon_review(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request =
        pruneguard_daemon::DaemonRequest::Review { base_ref: flags.changed_since.clone() };

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::ReviewResult { report }) => {
            eprintln!("(daemon-backed review)");
            print_daemon_report(&report, flags.format);
            let has_blocking = report
                .get("blockingFindings")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|arr| !arr.is_empty());
            Ok(Some(if has_blocking { ExitCode::from(1) } else { ExitCode::SUCCESS }))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon review failed: {message}");
            }
            tracing::debug!("daemon review returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon review failed: {err}");
            }
            tracing::debug!("daemon review request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed impact query.
fn try_daemon_impact(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    target: &str,
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::Impact {
        target: target.to_string(),
        focus: flags.focus.clone(),
    };

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::ImpactResult { report }) => {
            eprintln!("(daemon-backed impact)");
            print_daemon_report(&report, flags.format);
            Ok(Some(ExitCode::SUCCESS))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon impact failed: {message}");
            }
            tracing::debug!("daemon impact returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon impact failed: {err}");
            }
            tracing::debug!("daemon impact request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed explain query.
fn try_daemon_explain(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    query: &str,
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::Explain {
        query: query.to_string(),
        focus: flags.focus.clone(),
    };

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::ExplainResult { report }) => {
            eprintln!("(daemon-backed explain)");
            print_daemon_report(&report, flags.format);
            Ok(Some(ExitCode::SUCCESS))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon explain failed: {message}");
            }
            tracing::debug!("daemon explain returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon explain failed: {err}");
            }
            tracing::debug!("daemon explain request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed safe-delete evaluation.
fn try_daemon_safe_delete(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    targets: &[String],
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::SafeDelete { targets: targets.to_vec() };

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::SafeDeleteResult { report }) => {
            eprintln!("(daemon-backed safe-delete)");
            print_daemon_report(&report, flags.format);
            let has_blocked = report
                .get("blocked")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|arr| !arr.is_empty());
            let has_needs_review = report
                .get("needsReview")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|arr| !arr.is_empty());
            Ok(Some(if has_blocked || has_needs_review {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon safe-delete failed: {message}");
            }
            tracing::debug!("daemon safe-delete returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon safe-delete failed: {err}");
            }
            tracing::debug!("daemon safe-delete request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed fix-plan generation.
fn try_daemon_fix_plan(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    targets: &[String],
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::FixPlan { targets: targets.to_vec() };

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::FixPlanResult { report }) => {
            eprintln!("(daemon-backed fix-plan)");
            print_daemon_report(&report, flags.format);
            Ok(Some(ExitCode::SUCCESS))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon fix-plan failed: {message}");
            }
            tracing::debug!("daemon fix-plan returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon fix-plan failed: {err}");
            }
            tracing::debug!("daemon fix-plan request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed suggest-rules query.
fn try_daemon_suggest_rules(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::SuggestRules;

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::SuggestRulesResult { report }) => {
            eprintln!("(daemon-backed suggest-rules)");
            print_daemon_report(&report, flags.format);
            Ok(Some(ExitCode::SUCCESS))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon suggest-rules failed: {message}");
            }
            tracing::debug!("daemon suggest-rules returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon suggest-rules failed: {err}");
            }
            tracing::debug!("daemon suggest-rules request failed: {err}");
            Ok(None)
        }
    }
}

/// Try a daemon-backed compatibility report.
fn try_daemon_compatibility_report(
    cwd: &std::path::Path,
    mode: cli::DaemonMode,
    flags: &cli::GlobalFlags,
) -> miette::Result<Option<ExitCode>> {
    let Some(client) = try_daemon_client(cwd, mode)? else {
        return Ok(None);
    };

    let request = pruneguard_daemon::DaemonRequest::CompatibilityReport;

    match client.send_request(&request) {
        Ok(pruneguard_daemon::DaemonResponse::CompatibilityReportResult { report }) => {
            eprintln!("(daemon-backed compatibility-report)");
            print_daemon_report(&report, flags.format);
            Ok(Some(ExitCode::SUCCESS))
        }
        Ok(pruneguard_daemon::DaemonResponse::Error { message }) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon compatibility-report failed: {message}");
            }
            tracing::debug!("daemon compatibility-report returned error: {message}");
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(err) => {
            if matches!(mode, cli::DaemonMode::Required) {
                miette::bail!("daemon compatibility-report failed: {err}");
            }
            tracing::debug!("daemon compatibility-report request failed: {err}");
            Ok(None)
        }
    }
}

/// Print a daemon JSON report using the selected output format.
fn print_daemon_report(report: &serde_json::Value, format: cli::OutputFormat) {
    match format {
        cli::OutputFormat::Json
        | cli::OutputFormat::Text
        | cli::OutputFormat::Sarif
        | cli::OutputFormat::Dot => {
            // For all formats, print JSON for now -- the daemon returns raw
            // JSON reports.
            println!("{}", serde_json::to_string_pretty(report).expect("serialize report"));
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run_debug(
    cmd: cli::DebugCommand,
    config: &pruneguard_config::PruneguardConfig,
    profile: EntrypointProfile,
    format: cli::OutputFormat,
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
        cli::DebugCommand::Frameworks => {
            let report = pruneguard::debug_frameworks(&cwd, profile)?;
            if matches!(format, cli::OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&report).expect("serialize report"));
            } else {
                println!("--- framework debug report ---");
                println!();
                if report.detected_packs.is_empty() {
                    println!("no framework packs detected.");
                } else {
                    println!("detected framework packs:");
                    for pack in &report.detected_packs {
                        println!(
                            "  {} (confidence: {}, signals: {})",
                            pack.name,
                            pack.confidence,
                            pack.signals.join(", ")
                        );
                        for reason in &pack.reasons {
                            println!("    reason: {reason}");
                        }
                    }
                }
                if !report.all_entrypoints.is_empty() {
                    println!();
                    println!("contributed entrypoints:");
                    for ep in &report.all_entrypoints {
                        println!(
                            "  {} (framework: {}, kind: {}, heuristic: {})",
                            ep.path, ep.framework, ep.kind, ep.heuristic
                        );
                        println!("    reason: {}", ep.reason);
                    }
                }
                if !report.all_ignore_patterns.is_empty() {
                    println!();
                    println!("ignore patterns:");
                    for pattern in &report.all_ignore_patterns {
                        println!("  {pattern}");
                    }
                }
                if !report.all_classification_rules.is_empty() {
                    println!();
                    println!("classification rules:");
                    for rule in &report.all_classification_rules {
                        println!("  {} -> {}", rule.pattern, rule.classification);
                    }
                }
                if !report.heuristic_detections.is_empty() {
                    println!();
                    println!("heuristic detections:");
                    for detection in &report.heuristic_detections {
                        println!("  {detection}");
                    }
                }
                println!();
                println!("------------------------------");
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
            let schema_exists = std::path::Path::new(&schema_path).exists();
            let config_status = match pruneguard_config::PruneguardConfig::load(&cwd, None) {
                Ok(_) => "found".to_string(),
                Err(pruneguard_config::ConfigError::NotFound) => {
                    "not found (using defaults)".to_string()
                }
                Err(err) => format!("error: {err}"),
            };
            let is_ci = pruneguard_daemon::client::is_ci();
            println!("binary: {binary}");
            println!("platform: {}-{}", std::env::consts::OS, std::env::consts::ARCH);
            println!("version: {}", env!("CARGO_PKG_VERSION"));
            println!("cwd: {}", cwd.display());
            println!("schema_path: {schema_path}");
            println!("schema_exists: {schema_exists}");
            println!("config: {config_status}");
            println!("resolution_source: binary");
            println!("ci: {is_ci}");
            println!("default_execution_mode: {}", if is_ci { "oneshot" } else { "daemon" });

            // Report daemon status if available.
            let project_root = find_project_root_dir(&cwd);
            match pruneguard_daemon::DaemonClient::try_connect(&project_root) {
                Ok(Some(client)) => {
                    println!();
                    println!("daemon: running");
                    println!("daemon_pid: {}", client.pid());
                    println!("daemon_port: {}", client.port());
                    println!("daemon_version: {}", client.version());
                    match client.status() {
                        Ok(info) => {
                            println!("daemon_warm: {}", if info.index_warm { "yes" } else { "no" });
                            println!("daemon_graph_nodes: {}", info.graph_nodes);
                            println!("daemon_graph_edges: {}", info.graph_edges);
                            println!("daemon_watched_files: {}", info.watched_files);
                            println!("daemon_generation: {}", info.generation);
                            println!("daemon_last_update_ms: {}", info.last_update_ms);
                            if let Some(lag) = info.watcher_lag_ms {
                                println!("daemon_watcher_lag_ms: {lag}");
                            }
                            println!("daemon_uptime_secs: {}", info.uptime_secs);
                            println!("daemon_project_root: {}", info.project_root);
                            println!(
                                "daemon_pending_invalidations: {}",
                                info.pending_invalidations
                            );
                            if let Some(ref bp) = info.binary_path {
                                println!("daemon_binary_path: {bp}");
                            }
                            if let Some(ms) = info.initial_build_ms {
                                println!("daemon_initial_build_ms: {ms}");
                            }
                            if let Some(ms) = info.last_rebuild_ms {
                                println!("daemon_last_rebuild_ms: {ms}");
                            }
                        }
                        Err(err) => {
                            println!("daemon_status_error: {err}");
                        }
                    }
                }
                Ok(None) => {
                    println!();
                    println!("daemon: not running");
                }
                Err(err) => {
                    println!();
                    println!("daemon: error ({err})");
                }
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

                // Trust summary — prominent block so users can gauge reliability.
                println!();
                println!("--- trust summary ---");

                // Execution mode: daemon vs oneshot.
                let execution_mode = stats
                    .get("executionMode")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("oneshot");
                println!("mode: {execution_mode}");

                // Scope: full vs partial.
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

                // Baseline status.
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

                // Unresolved pressure.
                println!("unresolved specifiers: {unresolved}");
                if resolved > 0 && unresolved > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let pressure_pct = (unresolved as f64 / (resolved + unresolved) as f64) * 100.0;
                    if pressure_pct > 15.0 {
                        println!(
                            "  unresolved pressure: {pressure_pct:.1}% (HIGH) — many findings may be false positives"
                        );
                    } else if pressure_pct > 5.0 {
                        println!(
                            "  unresolved pressure: {pressure_pct:.1}% (moderate) — some findings may have lower accuracy"
                        );
                    } else {
                        println!("  unresolved pressure: {pressure_pct:.1}% (low)");
                    }
                }

                // Unresolved breakdown by reason.
                if let Some(by_reason) =
                    stats.get("unresolvedByReason").and_then(serde_json::Value::as_object)
                {
                    let missing = by_reason
                        .get("missingFile")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let unsupported = by_reason
                        .get("unsupportedSpecifier")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let tsconfig = by_reason
                        .get("tsconfigPathMiss")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let exports_miss = by_reason
                        .get("exportsConditionMiss")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let externalized = by_reason
                        .get("externalized")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if unresolved > 0 {
                        println!(
                            "  breakdown: missing={missing}, unsupported={unsupported}, tsconfig={tsconfig}, exports={exports_miss}, externalized={externalized}"
                        );
                    }
                }

                // Confidence counts.
                if let Some(confidence) =
                    stats.get("confidenceCounts").and_then(serde_json::Value::as_object)
                {
                    let high =
                        confidence.get("high").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    let medium =
                        confidence.get("medium").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    let low =
                        confidence.get("low").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    println!("confidence: high={high}, medium={medium}, low={low}",);
                    if high > 0 && low == 0 && medium == 0 {
                        println!("  all findings are high-confidence — safe to act on.");
                    } else if low > high + medium {
                        println!("  majority of findings are low-confidence — review carefully.");
                    }
                }

                // Daemon warm-index info.
                if let Some(index_warm) =
                    stats.get("indexWarm").and_then(serde_json::Value::as_bool)
                    && index_warm
                {
                    let age_ms =
                        stats.get("indexAgeMs").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    let reused_nodes = stats
                        .get("reusedGraphNodes")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    println!("warm index: reused {reused_nodes} nodes, age {age_ms}ms");
                }

                println!("---------------------");

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

fn print_review_text(report: &ReviewReport) {
    // --- Status line ---
    if report.blocking_findings.is_empty() {
        println!("Pruneguard: no blockers");
    } else {
        println!("Pruneguard: blockers found");
    }
    println!();

    // --- Blockers / advisories summary ---
    let blocking = report.blocking_findings.len();
    let advisory = report.advisory_findings.len();
    if blocking > 0 {
        println!("{blocking} blocking finding{}", if blocking == 1 { "" } else { "s" });
    }
    if advisory > 0 {
        println!("{advisory} advisory finding{}", if advisory == 1 { "" } else { "s" });
    }
    if blocking == 0 && advisory == 0 {
        println!("No findings.");
    }

    // --- Trust summary ---
    println!();
    println!("Trust");
    println!("  scope: {}", if report.trust.full_scope { "full" } else { "partial" });
    let cc = &report.trust.confidence_counts;
    println!("  confidence: {} high, {} medium, {} low", cc.high, cc.medium, cc.low);
    let pressure_label = if report.trust.unresolved_pressure > 0.15 {
        "high"
    } else if report.trust.unresolved_pressure > 0.05 {
        "medium"
    } else {
        "low"
    };
    println!("  unresolved pressure: {pressure_label}");
    println!("  baseline: {}", if report.trust.baseline_applied { "on" } else { "off" });
    if let Some(mode) = &report.trust.execution_mode {
        let mode_str = match mode {
            pruneguard_report::ExecutionMode::Daemon => "daemon",
            pruneguard_report::ExecutionMode::Oneshot => "one-shot",
        };
        println!("  mode: {mode_str}");
    }

    // --- Top blockers ---
    if !report.blocking_findings.is_empty() {
        println!();
        println!("Top blockers");
        for finding in report.blocking_findings.iter().take(10) {
            println!("  {}: {}", finding.code, finding.subject);
        }
        if blocking > 10 {
            println!("  ... and {} more", blocking - 10);
        }
    }

    // --- Advisories ---
    if !report.advisory_findings.is_empty() {
        println!();
        println!("Advisories");
        for finding in report.advisory_findings.iter().take(5) {
            println!("  {}: {}", finding.code, finding.subject);
        }
        if advisory > 5 {
            println!("  ... and {} more", advisory - 5);
        }
    }

    // --- Recommended next steps ---
    if !report.recommendations.is_empty() {
        println!();
        println!("Recommended next steps");
        for rec in &report.recommendations {
            println!("  {rec}");
        }
    } else if !report.blocking_findings.is_empty() {
        println!();
        println!("Recommended next steps");
        // Suggest concrete commands based on blocking findings.
        for finding in report.blocking_findings.iter().take(3) {
            if finding.code == "unused-file" {
                println!("  Run: pruneguard safe-delete {}", finding.subject);
            } else {
                println!("  Run: pruneguard explain {}", finding.id);
            }
        }
        if report.blocking_findings.is_empty() {
            println!("  Run: pruneguard scan   for detailed findings");
        }
    }

    // --- Latency ---
    if let Some(ms) = report.latency_ms {
        println!();
        println!("{ms}ms");
    }
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

#[allow(clippy::too_many_lines)]
fn run_daemon(cmd: &cli::DaemonCommand) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");
    let project_root = find_project_root_dir(&cwd);

    match cmd {
        cli::DaemonCommand::Start => {
            let config = load_config_or_default(&project_root, None)?;
            let server = pruneguard_daemon::DaemonServer::new(project_root, config);
            let rt = tokio::runtime::Runtime::new()
                .map_err(|err| miette::miette!("failed to create tokio runtime: {err}"))?;
            rt.block_on(async {
                server.run().await.map_err(|err| miette::miette!("daemon error: {err}"))
            })?;
            Ok(ExitCode::SUCCESS)
        }
        cli::DaemonCommand::Stop => {
            let metadata = pruneguard_daemon::DaemonMetadata::load(&project_root)
                .map_err(|err| miette::miette!("failed to load daemon metadata: {err}"))?;
            if let Some(meta) = metadata {
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|err| miette::miette!("failed to create tokio runtime: {err}"))?;
                rt.block_on(async { send_daemon_shutdown(meta.port, &meta.token).await })?;
                eprintln!("daemon stopped");
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("no running daemon found");
                Ok(ExitCode::from(1))
            }
        }
        cli::DaemonCommand::Status => {
            match pruneguard_daemon::DaemonClient::try_connect(&project_root) {
                Ok(Some(client)) => {
                    println!("pid: {}", client.pid());
                    println!("port: {}", client.port());
                    println!("version: {}", client.version());
                    println!("execution_mode: daemon");
                    println!("project_root: {}", project_root.display());
                    match client.status() {
                        Ok(info) => {
                            println!("warm: {}", if info.index_warm { "yes" } else { "no" });
                            println!("graph_nodes: {}", info.graph_nodes);
                            println!("graph_edges: {}", info.graph_edges);
                            println!("watched_files: {}", info.watched_files);
                            println!("generation: {}", info.generation);
                            println!("last_update_ms: {}", info.last_update_ms);
                            if let Some(lag) = info.watcher_lag_ms {
                                println!("watcher_lag_ms: {lag}");
                            }
                            println!("pending_invalidations: {}", info.pending_invalidations);
                            println!("uptime_secs: {}", info.uptime_secs);
                            if let Some(ref bp) = info.binary_path {
                                println!("binary_path: {bp}");
                            }
                            if let Some(ms) = info.initial_build_ms {
                                println!("initial_build_ms: {ms}");
                            }
                            if let Some(ms) = info.last_rebuild_ms {
                                println!("last_rebuild_ms: {ms}");
                            }
                            println!("incremental_rebuilds: {}", info.incremental_rebuilds);
                            println!("total_invalidations: {}", info.total_invalidations);
                            println!("config_change_pending: {}", info.config_change_pending);
                        }
                        Err(err) => {
                            eprintln!("failed to query daemon status: {err}");
                        }
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Ok(None) => {
                    println!("no running daemon");
                    Ok(ExitCode::from(1))
                }
                Err(err) => {
                    eprintln!("failed to connect to daemon: {err}");
                    Ok(ExitCode::from(1))
                }
            }
        }
        cli::DaemonCommand::Restart => {
            // Stop any existing daemon, then start a new one.
            if let Ok(Some(meta)) = pruneguard_daemon::DaemonMetadata::load(&project_root) {
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|err| miette::miette!("failed to create tokio runtime: {err}"))?;
                rt.block_on(async { send_daemon_shutdown(meta.port, &meta.token).await }).ok();
                eprintln!("previous daemon stopped");
            }
            let config = load_config_or_default(&project_root, None)?;
            let server = pruneguard_daemon::DaemonServer::new(project_root, config);
            let rt = tokio::runtime::Runtime::new()
                .map_err(|err| miette::miette!("failed to create tokio runtime: {err}"))?;
            rt.block_on(async {
                server.run().await.map_err(|err| miette::miette!("daemon error: {err}"))
            })?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn find_project_root_dir(cwd: &std::path::Path) -> std::path::PathBuf {
    let mut dir = cwd.to_path_buf();
    loop {
        if dir.join("pruneguard.json").exists() || dir.join("package.json").exists() {
            return dir;
        }
        if !dir.pop() {
            return cwd.to_path_buf();
        }
    }
}

async fn send_daemon_shutdown(port: u16, token: &str) -> miette::Result<()> {
    use pruneguard_daemon::protocol::{read_frame, write_frame};
    use tokio::net::TcpStream;

    let stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .map_err(|err| miette::miette!("failed to connect to daemon: {err}"))?;

    let (mut reader, mut writer) = stream.into_split();

    // Send auth token as first frame.
    write_frame(&mut writer, token.as_bytes())
        .await
        .map_err(|err| miette::miette!("failed to write to daemon: {err}"))?;

    // Send shutdown request.
    let request = pruneguard_daemon::DaemonRequest::Shutdown;
    let payload = serde_json::to_vec(&request).expect("serialize request");
    write_frame(&mut writer, &payload)
        .await
        .map_err(|err| miette::miette!("failed to write to daemon: {err}"))?;

    // Read response.
    if let Some(_resp) = read_frame(&mut reader)
        .await
        .map_err(|err| miette::miette!("failed to read from daemon: {err}"))?
    {
        // Response received, shutdown acknowledged.
    }

    Ok(())
}
