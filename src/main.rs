use std::path::Path;
use std::process;

use clap::{Parser, Subcommand};
use colored::Colorize;

use pgroller::cli::{
    run_baseline, run_init, run_migrate, run_new, run_rollback, run_status, run_test, run_verify,
    InitSource,
};
use pgroller::config::load_config;
use pgroller::ui;

#[derive(Parser)]
#[command(name = "pgroller", version, about = "SQL migration rollback validator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Test migration rollbacks for round-trip safety
    Test {
        /// Path to config file (default: pgroller.toml or .pgroller.toml)
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Validate migration rollbacks (alias for test)
    Validate {
        /// Path to config file (default: pgroller.toml or .pgroller.toml)
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Collapse all migrations into a new baseline
    Baseline {
        /// Path to config file (default: pgroller.toml or .pgroller.toml)
        #[arg(short, long)]
        config: Option<String>,

        /// Preview what would happen without making changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Show migration status against a database
    Status {
        /// PostgreSQL connection URI (required)
        #[arg(long)]
        database: String,
        /// Path to config file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Apply pending migrations
    Migrate {
        /// PostgreSQL connection URI (required)
        #[arg(long)]
        database: String,
        /// Preview without applying
        #[arg(long)]
        dry_run: bool,
        /// Verify schema after migration
        #[arg(long)]
        verify: bool,
        /// Skip confirmation prompt
        #[arg(long)]
        accept: bool,
        /// Path to config file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Rollback applied migrations
    Rollback {
        /// PostgreSQL connection URI (required)
        #[arg(long)]
        database: String,
        /// Number of migrations to rollback (default: 1)
        #[arg(long, default_value = "1")]
        steps: usize,
        /// Preview without applying
        #[arg(long)]
        dry_run: bool,
        /// Verify schema after rollback (default: true)
        #[arg(long, default_value = "true")]
        verify: bool,
        /// Skip confirmation prompt
        #[arg(long)]
        accept: bool,
        /// Path to config file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Verify a database schema matches the expected migration state
    Verify {
        /// PostgreSQL connection URI (required)
        #[arg(long)]
        database: String,
        /// Path to config file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Create a new migration folder
    New {
        /// Migration name (spaces and mixed case allowed, will be normalized)
        name: String,
        /// Skip confirmation prompt
        #[arg(long)]
        accept: bool,
        /// Path to config file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Initialize a new pgroller project
    Init {
        /// Directory to create the project in
        #[arg(default_value = "./db/migrations")]
        path: String,

        /// Initialize baseline from a live database (PostgreSQL URI)
        #[arg(long)]
        from_database: Option<String>,

        /// Initialize baseline from a pg_dump file
        #[arg(long)]
        from_dump: Option<String>,

        /// Always strip DML from dump file (non-interactive)
        #[arg(long, conflicts_with = "keep_dml")]
        strip_dml: bool,

        /// Always keep DML in dump file (non-interactive)
        #[arg(long, conflicts_with = "strip_dml")]
        keep_dml: bool,

        /// PostgreSQL version for pgroller.toml
        #[arg(long, default_value = "15")]
        postgres_version: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    ui::print_banner();

    match cli.command {
        Commands::Test { ref config } | Commands::Validate { ref config } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            let report = match run_test(&cfg).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            if report.is_success() {
                process::exit(0);
            } else {
                process::exit(1);
            }
        }
        Commands::Baseline {
            ref config,
            dry_run,
        } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            match run_baseline(&cfg, dry_run).await {
                Ok(_) => process::exit(0),
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
        Commands::Status {
            ref database,
            ref config,
        } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            match run_status(&cfg, database).await {
                Ok(_) => process::exit(0),
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
        Commands::Migrate {
            ref database,
            dry_run,
            verify,
            accept,
            ref config,
        } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            match run_migrate(&cfg, database, dry_run, accept).await {
                Ok(_) => {
                    if verify && !dry_run {
                        if let Err(e) = run_verify(&cfg, database).await {
                            eprintln!("\n  {} {}", "✗".red().bold(), e);
                            process::exit(1);
                        }
                    }
                    process::exit(0);
                }
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
        Commands::Rollback {
            ref database,
            steps,
            dry_run,
            verify,
            accept,
            ref config,
        } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            match run_rollback(&cfg, database, steps, dry_run, accept).await {
                Ok(_) => {
                    if verify && !dry_run {
                        if let Err(e) = run_verify(&cfg, database).await {
                            eprintln!("\n  {} {}", "✗".red().bold(), e);
                            process::exit(1);
                        }
                    }
                    process::exit(0);
                }
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
        Commands::Verify {
            ref database,
            ref config,
        } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            match run_verify(&cfg, database).await {
                Ok(_) => process::exit(0),
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
        Commands::New {
            ref name,
            accept,
            ref config,
        } => {
            let config_path = config.as_deref();
            let cfg = match load_config(config_path.map(Path::new)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            };

            match run_new(&cfg, name, accept) {
                Ok(_) => process::exit(0),
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
        Commands::Init {
            ref path,
            ref from_database,
            ref from_dump,
            strip_dml,
            keep_dml,
            ref postgres_version,
        } => {
            let source = if let Some(uri) = from_database {
                InitSource::Database(uri.clone())
            } else if let Some(file) = from_dump {
                InitSource::Dump {
                    path: file.clone(),
                    strip_dml,
                    keep_dml,
                }
            } else {
                InitSource::Fresh
            };

            match run_init(Path::new(path), source, postgres_version).await {
                Ok(_) => process::exit(0),
                Err(e) => {
                    eprintln!("\n  {} {}", "✗".red().bold(), e);
                    process::exit(1);
                }
            }
        }
    }
}
