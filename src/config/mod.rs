use std::path::Path;

use serde::Deserialize;

use crate::{PgrollerError, Result};

#[derive(Debug, Deserialize, Clone)]
pub struct PgrollerConfig {
    #[serde(default = "default_migrations")]
    pub migrations: MigrationsConfig,
    #[serde(default = "default_database")]
    pub database: DatabaseConfig,
    #[serde(default = "default_test")]
    pub test: TestConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MigrationsConfig {
    #[serde(default = "default_dir")]
    pub dir: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    #[serde(default = "default_postgres_version")]
    pub postgres_version: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_schema")]
    pub schema: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TestConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_continue_on_failure")]
    pub continue_on_failure: bool,
    /// How to reset between tests: "savepoint" (fast) or "drop_schema" (safe)
    #[serde(default = "default_reset_strategy")]
    pub reset_strategy: String,
}

fn default_migrations() -> MigrationsConfig {
    MigrationsConfig { dir: default_dir() }
}

fn default_database() -> DatabaseConfig {
    DatabaseConfig {
        postgres_version: default_postgres_version(),
        extensions: Vec::new(),
        schema: default_schema(),
    }
}

fn default_test() -> TestConfig {
    TestConfig {
        timeout: default_timeout(),
        continue_on_failure: default_continue_on_failure(),
        reset_strategy: default_reset_strategy(),
    }
}

fn default_continue_on_failure() -> bool {
    true
}

fn default_reset_strategy() -> String {
    "drop_schema".to_string()
}

fn default_dir() -> String {
    "db/migrations".to_string()
}

fn default_postgres_version() -> String {
    "15".to_string()
}

fn default_schema() -> String {
    "public".to_string()
}

fn default_timeout() -> u64 {
    30
}

impl Default for PgrollerConfig {
    fn default() -> Self {
        Self {
            migrations: default_migrations(),
            database: default_database(),
            test: default_test(),
        }
    }
}

/// Load configuration from a TOML file.
///
/// If `path` is provided, loads from that file.
/// Otherwise, tries `./pgroller.toml`, then `./.pgroller.toml`.
/// If neither exists, returns default configuration.
pub fn load_config(path: Option<&Path>) -> Result<PgrollerConfig> {
    let config_path = if let Some(p) = path {
        if !p.exists() {
            return Err(PgrollerError::Config(format!(
                "Config file not found: {}",
                p.display()
            )));
        }
        Some(p.to_path_buf())
    } else {
        let candidates = [Path::new("./pgroller.toml"), Path::new("./.pgroller.toml")];
        candidates
            .iter()
            .find(|p| p.exists())
            .map(|p| p.to_path_buf())
    };

    match config_path {
        Some(p) => {
            let content = std::fs::read_to_string(&p)?;
            let config: PgrollerConfig = toml::from_str(&content)?;
            Ok(config)
        }
        None => Ok(PgrollerConfig::default()),
    }
}
