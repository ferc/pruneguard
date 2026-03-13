#![allow(dead_code)] // Parser-only fields may be unused in specific command paths.

use bpaf::{OptionParser, Parser, construct, long, positional, pure, short};
use std::path::PathBuf;

/// Global flags shared across all commands.
#[derive(Debug, Clone)]
pub struct GlobalFlags {
    pub format: OutputFormat,
    pub profile: Profile,
    pub changed_since: Option<String>,
    pub focus: Option<String>,
    pub severity: Severity,
    pub no_cache: bool,
    pub no_baseline: bool,
    pub require_full_scope: bool,
    pub max_findings: Option<usize>,
    pub daemon: DaemonMode,
    pub semantic: SemanticCliMode,
    pub semantic_max_overhead_pct: Option<u8>,
    pub semantic_max_wall_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum DaemonMode {
    #[default]
    Auto,
    Off,
    Required,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum SemanticCliMode {
    #[default]
    Auto,
    Off,
    Required,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Sarif,
    Dot,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum Profile {
    Production,
    Development,
    #[default]
    All,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum Severity {
    Error,
    #[default]
    Warn,
    Info,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub config: Option<PathBuf>,
    pub global: GlobalFlags,
    pub command: Command,
}

#[derive(Debug, Clone)]
pub enum Command {
    Scan {
        paths: Vec<PathBuf>,
    },
    Impact {
        target: String,
    },
    Explain {
        query: String,
    },
    Review {
        strict_trust: bool,
    },
    SafeDelete {
        targets: Vec<String>,
    },
    FixPlan {
        targets: Vec<String>,
    },
    SuggestRules,
    Init,
    PrintConfig,
    CompatibilityReport,
    Debug(DebugCommand),
    Migrate(MigrateCommand),
    Daemon(DaemonCommand),
    Bench(BenchCommand),
    /// Bare positional paths without explicit subcommand — rejected with guidance.
    BarePathsError {
        paths: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub enum BenchCommand {
    Replacement { corpus: Option<PathBuf> },
    Performance { corpus: Option<PathBuf>, iterations: Option<usize> },
    Compare { tool: String, corpus: Option<PathBuf> },
}

#[derive(Debug, Clone)]
pub enum DaemonCommand {
    Start,
    Stop,
    Restart,
    Status,
}

#[derive(Debug, Clone)]
pub enum DebugCommand {
    Resolve { from: PathBuf, specifier: String },
    Entrypoints,
    Runtime,
    Frameworks,
}

#[derive(Debug, Clone)]
pub enum MigrateCommand {
    Knip { file: Option<PathBuf> },
    Depcruise { file: Option<PathBuf>, node: bool },
}

fn parse_format(s: &str) -> Result<OutputFormat, String> {
    match s {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        "sarif" => Ok(OutputFormat::Sarif),
        "dot" => Ok(OutputFormat::Dot),
        other => Err(format!("unknown format: {other}")),
    }
}

fn parse_profile(s: &str) -> Result<Profile, String> {
    match s {
        "production" => Ok(Profile::Production),
        "development" => Ok(Profile::Development),
        "all" => Ok(Profile::All),
        other => Err(format!("unknown profile: {other}")),
    }
}

fn parse_severity(s: &str) -> Result<Severity, String> {
    match s {
        "error" => Ok(Severity::Error),
        "warn" => Ok(Severity::Warn),
        "info" => Ok(Severity::Info),
        other => Err(format!("unknown severity: {other}")),
    }
}

fn parse_daemon_mode(s: &str) -> Result<DaemonMode, String> {
    match s {
        "auto" => Ok(DaemonMode::Auto),
        "off" => Ok(DaemonMode::Off),
        "required" => Ok(DaemonMode::Required),
        other => Err(format!("unknown daemon mode: {other}")),
    }
}

fn parse_semantic_mode(s: &str) -> Result<SemanticCliMode, String> {
    match s {
        "auto" => Ok(SemanticCliMode::Auto),
        "off" => Ok(SemanticCliMode::Off),
        "required" => Ok(SemanticCliMode::Required),
        other => Err(format!("unknown semantic mode: {other}")),
    }
}

fn global_flags() -> impl Parser<GlobalFlags> {
    let format = long("format")
        .help("Output format: text|json|sarif|dot")
        .argument::<String>("FORMAT")
        .parse(|s| parse_format(&s))
        .fallback(OutputFormat::Text);
    let profile = long("profile")
        .help("Analysis profile: production|development|all")
        .argument::<String>("PROFILE")
        .parse(|s| parse_profile(&s))
        .fallback(Profile::All);
    let changed_since = long("changed-since")
        .help("Only analyze files changed since this git ref")
        .argument::<String>("REF")
        .optional();
    let focus = long("focus")
        .help("Focus analysis on files matching this glob")
        .argument::<String>("GLOB")
        .optional();
    let severity = long("severity")
        .help("Minimum severity to report: error|warn|info")
        .argument::<String>("SEVERITY")
        .parse(|s| parse_severity(&s))
        .fallback(Severity::Warn);
    let no_cache = long("no-cache").help("Disable incremental cache").switch();
    let no_baseline = long("no-baseline").help("Disable baseline auto-discovery").switch();
    let require_full_scope = long("require-full-scope")
        .help("Fail partial-scope scan runs when dead-code analyzers are active")
        .switch();
    let max_findings = long("max-findings")
        .help("Maximum number of findings to report")
        .argument::<usize>("N")
        .optional();
    let daemon = long("daemon")
        .help("Daemon mode: auto|off|required")
        .argument::<String>("MODE")
        .parse(|s| parse_daemon_mode(&s))
        .fallback(DaemonMode::Auto);
    let semantic = long("semantic")
        .help("Semantic helper mode: off|auto|required")
        .argument::<String>("MODE")
        .parse(|s| parse_semantic_mode(&s))
        .fallback(SemanticCliMode::Auto);
    let semantic_max_overhead_pct = long("semantic-max-overhead-pct")
        .help("Maximum cold-scan overhead percentage for semantic helper (0-100)")
        .argument::<u8>("N")
        .optional();
    let semantic_max_wall_ms = long("semantic-max-wall-ms")
        .help("Maximum wall-clock milliseconds for semantic helper")
        .argument::<u64>("MS")
        .optional();

    construct!(GlobalFlags {
        format,
        profile,
        changed_since,
        focus,
        severity,
        no_cache,
        no_baseline,
        require_full_scope,
        max_findings,
        daemon,
        semantic,
        semantic_max_overhead_pct,
        semantic_max_wall_ms,
    })
}

fn scan_command() -> impl Parser<Command> {
    let paths = positional::<PathBuf>("PATHS").help("Paths to analyze").many();
    construct!(Command::Scan { paths })
}

fn impact_command() -> impl Parser<Command> {
    let target = positional::<String>("TARGET").help("File or export to analyze impact for");
    construct!(Command::Impact { target })
}

fn explain_command() -> impl Parser<Command> {
    let query = positional::<String>("QUERY").help("Finding ID or path to explain");
    construct!(Command::Explain { query })
}

fn review_command() -> impl Parser<Command> {
    let strict_trust = long("strict-trust")
        .help("Only block on full-scope, high-confidence, low-pressure findings; downgrade others to advisory")
        .switch();
    construct!(Command::Review { strict_trust })
}

fn compatibility_report_command() -> impl Parser<Command> {
    pure(Command::CompatibilityReport)
}

fn safe_delete_command() -> impl Parser<Command> {
    let targets = positional::<String>("TARGETS")
        .help("Files or exports to evaluate for safe deletion")
        .many();
    construct!(Command::SafeDelete { targets })
}

fn fix_plan_command() -> impl Parser<Command> {
    let targets = positional::<String>("TARGETS")
        .help("Finding IDs, file paths, or export names to generate fix plans for")
        .many();
    construct!(Command::FixPlan { targets })
}

fn suggest_rules_command() -> impl Parser<Command> {
    pure(Command::SuggestRules)
}

fn init_command() -> impl Parser<Command> {
    pure(Command::Init)
}

fn print_config_command() -> impl Parser<Command> {
    pure(Command::PrintConfig)
}

fn debug_resolve_subcommand() -> impl Parser<DebugCommand> {
    // Named argument must come before positional for bpaf
    let from = long("from").help("File to resolve from").argument::<PathBuf>("FILE");
    let specifier = positional::<String>("SPECIFIER").help("Module specifier to resolve");
    construct!(DebugCommand::Resolve { from, specifier })
}

fn debug_entrypoints_subcommand() -> impl Parser<DebugCommand> {
    pure(DebugCommand::Entrypoints)
}

fn debug_runtime_subcommand() -> impl Parser<DebugCommand> {
    pure(DebugCommand::Runtime)
}

fn debug_frameworks_subcommand() -> impl Parser<DebugCommand> {
    pure(DebugCommand::Frameworks)
}

fn debug_command() -> impl Parser<Command> {
    let resolve =
        debug_resolve_subcommand().to_options().descr("Debug module resolution").command("resolve");
    let entrypoints = debug_entrypoints_subcommand()
        .to_options()
        .descr("Debug entrypoint detection")
        .command("entrypoints");
    let runtime = debug_runtime_subcommand()
        .to_options()
        .descr("Print runtime diagnostic info")
        .command("runtime");
    let frameworks = debug_frameworks_subcommand()
        .to_options()
        .descr("Debug framework detection and contributed rules")
        .command("frameworks");

    construct!([resolve, entrypoints, runtime, frameworks]).map(Command::Debug)
}

fn migrate_knip_subcommand() -> impl Parser<MigrateCommand> {
    let file = positional::<PathBuf>("FILE").help("Path to knip config file").optional();
    construct!(MigrateCommand::Knip { file })
}

fn migrate_depcruise_subcommand() -> impl Parser<MigrateCommand> {
    let node = long("node").help("Use Node.js to evaluate dynamic config").switch();
    let file =
        positional::<PathBuf>("FILE").help("Path to dependency-cruiser config file").optional();
    // Named flags must come before positionals in construct! for bpaf
    construct!(MigrateCommand::Depcruise { node, file })
}

fn migrate_command() -> impl Parser<Command> {
    let knip = migrate_knip_subcommand()
        .to_options()
        .descr("Migrate from knip configuration")
        .command("knip");
    let depcruise = migrate_depcruise_subcommand()
        .to_options()
        .descr("Migrate from dependency-cruiser configuration")
        .command("depcruise");

    construct!([knip, depcruise]).map(Command::Migrate)
}

fn bench_replacement_subcommand() -> impl Parser<BenchCommand> {
    let corpus =
        positional::<PathBuf>("CORPUS").help("Path to parity fixture corpus directory").optional();
    construct!(BenchCommand::Replacement { corpus })
}

fn bench_performance_subcommand() -> impl Parser<BenchCommand> {
    let corpus =
        positional::<PathBuf>("CORPUS").help("Path to benchmark corpus directory").optional();
    let iterations = long("iterations")
        .help("Number of iterations for statistical accuracy")
        .argument::<usize>("N")
        .optional();
    construct!(BenchCommand::Performance { corpus, iterations })
}

fn bench_compare_subcommand() -> impl Parser<BenchCommand> {
    let tool = long("tool").help("Tool to compare against (e.g. knip)").argument::<String>("TOOL");
    let corpus =
        positional::<PathBuf>("CORPUS").help("Path to benchmark corpus directory").optional();
    // Named flags must come before positionals in construct! for bpaf
    construct!(BenchCommand::Compare { tool, corpus })
}

fn bench_command() -> impl Parser<Command> {
    let replacement = bench_replacement_subcommand()
        .to_options()
        .descr("Compute weighted replacement score against parity corpus")
        .command("replacement");
    let performance = bench_performance_subcommand()
        .to_options()
        .descr("Run cold-scan performance benchmarks against corpus repos")
        .command("performance");
    let compare = bench_compare_subcommand()
        .to_options()
        .descr("Compare pruneguard results against another tool")
        .command("compare");

    construct!([replacement, performance, compare]).map(Command::Bench)
}

fn daemon_start_subcommand() -> impl Parser<DaemonCommand> {
    pure(DaemonCommand::Start)
}

fn daemon_stop_subcommand() -> impl Parser<DaemonCommand> {
    pure(DaemonCommand::Stop)
}

fn daemon_restart_subcommand() -> impl Parser<DaemonCommand> {
    pure(DaemonCommand::Restart)
}

fn daemon_status_subcommand() -> impl Parser<DaemonCommand> {
    pure(DaemonCommand::Status)
}

fn daemon_command() -> impl Parser<Command> {
    let start = daemon_start_subcommand()
        .to_options()
        .descr("Start the pruneguard daemon")
        .command("start");
    let stop =
        daemon_stop_subcommand().to_options().descr("Stop the running daemon").command("stop");
    let restart = daemon_restart_subcommand()
        .to_options()
        .descr("Restart the daemon (stop then start)")
        .command("restart");
    let status =
        daemon_status_subcommand().to_options().descr("Show daemon status").command("status");

    construct!([start, stop, restart, status]).map(Command::Daemon)
}

/// Catch bare positional paths (no subcommand) and return a helpful error.
fn bare_paths_catcher() -> impl Parser<Command> {
    let paths = positional::<String>("PATH")
        .many()
        .guard(|p: &Vec<String>| !p.is_empty(), "expected at least one path");
    paths.map(|paths| Command::BarePathsError { paths })
}

fn command_parser() -> impl Parser<Command> {
    // --- Daily use ---
    let review =
        review_command().to_options().descr("Review your repo or branch").command("review");
    let scan =
        scan_command().to_options().descr("Full repo scan with detailed findings").command("scan");
    let safe_delete = safe_delete_command()
        .to_options()
        .descr("Check if files or exports are safe to remove")
        .command("safe-delete");
    let fix_plan =
        fix_plan_command().to_options().descr("Generate a remediation plan").command("fix-plan");

    // --- Investigation ---
    let impact =
        impact_command().to_options().descr("Analyze blast radius for a target").command("impact");
    let explain = explain_command()
        .to_options()
        .descr("Explain a finding with proof chain")
        .command("explain");

    // --- Policy and governance ---
    let suggest_rules = suggest_rules_command()
        .to_options()
        .descr("Auto-suggest governance rules")
        .command("suggest-rules");
    let compatibility_report = compatibility_report_command()
        .to_options()
        .descr("Report framework compatibility")
        .command("compatibility-report");

    // --- Setup ---
    let init =
        init_command().to_options().descr("Generate a minimal pruneguard.json").command("init");
    let print_config = print_config_command()
        .to_options()
        .descr("Print resolved configuration")
        .command("print-config");

    // --- Debugging and migration ---
    let debug = debug_command()
        .to_options()
        .descr("Debug resolution, entrypoints, and frameworks")
        .command("debug");
    let migrate = migrate_command()
        .to_options()
        .descr("Migrate from knip or dependency-cruiser")
        .command("migrate");
    let daemon =
        daemon_command().to_options().descr("Manage the background daemon").command("daemon");
    let bench = bench_command()
        .to_options()
        .descr("Run benchmarks: replacement scoring, performance, and tool comparison")
        .command("bench");

    // Catch bare positional paths and give a helpful error message.
    let bare_paths = bare_paths_catcher();

    // Default to review when no subcommand is given.
    let default_review = review_command();

    construct!([
        review,
        scan,
        safe_delete,
        fix_plan,
        impact,
        explain,
        suggest_rules,
        compatibility_report,
        init,
        print_config,
        debug,
        migrate,
        daemon,
        bench,
        bare_paths,
        default_review
    ])
}

pub fn options() -> OptionParser<Options> {
    let config = short('c')
        .long("config")
        .help("Path to config file")
        .argument::<PathBuf>("FILE")
        .optional();
    let global = global_flags();
    let command = command_parser();

    construct!(Options { config, global, command })
        .to_options()
        .descr("pruneguard - Find unused code, boundary violations, and architecture issues in JS/TS repos")
        .version(env!("CARGO_PKG_VERSION"))
}
