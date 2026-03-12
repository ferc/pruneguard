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
    pub max_findings: Option<usize>,
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
    Scan { paths: Vec<PathBuf> },
    Impact { target: String },
    Explain { query: String },
    Init,
    PrintConfig,
    Debug(DebugCommand),
    Migrate(MigrateCommand),
}

#[derive(Debug, Clone)]
pub enum DebugCommand {
    Resolve { from: PathBuf, specifier: String },
    Entrypoints,
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
    let max_findings = long("max-findings")
        .help("Maximum number of findings to report")
        .argument::<usize>("N")
        .optional();

    construct!(GlobalFlags {
        format,
        profile,
        changed_since,
        focus,
        severity,
        no_cache,
        max_findings,
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

fn debug_command() -> impl Parser<Command> {
    let resolve =
        debug_resolve_subcommand().to_options().descr("Debug module resolution").command("resolve");
    let entrypoints = debug_entrypoints_subcommand()
        .to_options()
        .descr("Debug entrypoint detection")
        .command("entrypoints");

    construct!([resolve, entrypoints]).map(Command::Debug)
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

fn command_parser() -> impl Parser<Command> {
    let scan = scan_command().to_options().descr("Analyze the repository").command("scan");
    let impact =
        impact_command().to_options().descr("Compute blast radius for a target").command("impact");
    let explain =
        explain_command().to_options().descr("Explain a finding or path").command("explain");
    let init = init_command().to_options().descr("Generate an oxgraph.json config").command("init");
    let print_config = print_config_command()
        .to_options()
        .descr("Print resolved configuration")
        .command("print-config");
    let debug = debug_command()
        .to_options()
        .descr("Debug tools for resolution and entrypoints")
        .command("debug");
    let migrate =
        migrate_command().to_options().descr("Migrate from other tools").command("migrate");

    // Default to scan when no subcommand is given
    let default_scan = scan_command();

    construct!([scan, impact, explain, init, print_config, debug, migrate, default_scan])
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
        .descr("oxgraph - Repo truth engine for JS/TS monorepos")
        .version(env!("CARGO_PKG_VERSION"))
}
