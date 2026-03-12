#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

mod cli;

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
    match options.command {
        cli::Command::Scan { paths } => {
            let cwd = std::env::current_dir().expect("failed to get current directory");
            let config = oxgraph_config::OxgraphConfig::load(&cwd, options.config.as_deref())?;
            let _report = oxgraph::scan(&cwd, &config, &paths)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Impact { target } => {
            let cwd = std::env::current_dir().expect("failed to get current directory");
            let config = oxgraph_config::OxgraphConfig::load(&cwd, options.config.as_deref())?;
            let _report = oxgraph::impact(&cwd, &config, &target)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Explain { query } => {
            let cwd = std::env::current_dir().expect("failed to get current directory");
            let config = oxgraph_config::OxgraphConfig::load(&cwd, options.config.as_deref())?;
            let _report = oxgraph::explain(&cwd, &config, &query)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Init => {
            oxgraph_config::OxgraphConfig::init()?;
            eprintln!("Created oxgraph.json");
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::PrintConfig => {
            let cwd = std::env::current_dir().expect("failed to get current directory");
            let config = oxgraph_config::OxgraphConfig::load(&cwd, options.config.as_deref())?;
            let json = serde_json::to_string_pretty(&config).expect("failed to serialize config");
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Debug(debug_cmd) => run_debug(debug_cmd, options.config.as_deref()),
        cli::Command::Migrate(ref migrate_cmd) => run_migrate(migrate_cmd),
    }
}

fn run_debug(
    cmd: cli::DebugCommand,
    config_path: Option<&std::path::Path>,
) -> miette::Result<ExitCode> {
    let cwd = std::env::current_dir().expect("failed to get current directory");
    let config = oxgraph_config::OxgraphConfig::load(&cwd, config_path)?;

    match cmd {
        cli::DebugCommand::Resolve { specifier, from } => {
            let result = oxgraph_resolver::debug_resolve(&config.resolver, &specifier, &from);
            println!("{result}");
            Ok(ExitCode::SUCCESS)
        }
        cli::DebugCommand::Entrypoints => {
            let entrypoints = oxgraph::debug_entrypoints(&cwd, &config)?;
            for ep in &entrypoints {
                println!("{ep}");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_migrate(cmd: &cli::MigrateCommand) -> miette::Result<ExitCode> {
    match cmd {
        cli::MigrateCommand::Knip { .. } => {
            miette::bail!("migrate knip is not yet implemented");
        }
        cli::MigrateCommand::Depcruise { .. } => {
            miette::bail!("migrate depcruise is not yet implemented");
        }
    }
}
