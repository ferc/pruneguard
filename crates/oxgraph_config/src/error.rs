use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum ConfigError {
    #[error("config file not found")]
    #[diagnostic(help(
        "Run `oxgraph init` to create a config file, or use `--config` to specify one"
    ))]
    NotFound,

    #[error("failed to read config file: {path}")]
    ReadError {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config file: {path}")]
    ParseError {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to write config file")]
    WriteError(#[from] std::io::Error),
}
