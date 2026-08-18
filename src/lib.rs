pub mod cli;
pub mod config;
pub mod container;
pub mod differ;
pub mod discovery;
pub mod executor;
pub mod matcher;
pub mod parser;
pub mod sql_parser;
pub mod testing;
pub mod ui;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum PgrollerError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Discovery error: {0}")]
    Discovery(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Container error: {0}")]
    Container(String),

    #[error("SQL execution error: {0}")]
    Execution(String),

    #[error("Diff error: {0}")]
    Diff(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Database error: {0}")]
    Database(#[from] tokio_postgres::Error),
}

pub type Result<T> = std::result::Result<T, PgrollerError>;
