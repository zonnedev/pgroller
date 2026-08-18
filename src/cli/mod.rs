use std::path::Path;

use colored::Colorize;
use console;

use crate::config::PgrollerConfig;
use crate::container::PgContainer;
use crate::differ::{diff_data, diff_schemas, DataDiff, SchemaDiff};
use crate::discovery::{discover_migrations, Migration};
use crate::executor::{execute_file, execute_query, PersistentConn};
use crate::matcher::{match_diffs, UncoveredDiff};
use crate::parser::{parse_annotations, NoDataRollback, NoSchemaRollback};
use crate::ui::{self, TestProgress};
use crate::{PgrollerError, Result};

/// SQL to create the pgroller_test schema with assertion functions.
pub const PGROLLER_TEST_SCHEMA: &str = r#"
CREATE SCHEMA IF NOT EXISTS pgroller_test;

CREATE OR REPLACE FUNCTION pgroller_test.assert_equal(description text, actual anyelement, expected anyelement)
RETURNS void AS $$
BEGIN
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION '[test-assertion] %: got ''%'' expected ''%''', description, actual, expected;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION pgroller_test.assert_true(description text, condition boolean)
RETURNS void AS $$
BEGIN
    IF condition IS NOT TRUE THEN
        RAISE EXCEPTION '[test-assertion] %: expected true', description;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION pgroller_test.assert_false(description text, condition boolean)
RETURNS void AS $$
BEGIN
    IF condition IS NOT FALSE THEN
        RAISE EXCEPTION '[test-assertion] %: expected false', description;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION pgroller_test.assert_null(description text, value anyelement)
RETURNS void AS $$
BEGIN
    IF value IS NOT NULL THEN
        RAISE EXCEPTION '[test-assertion] %: expected NULL got ''%''', description, value;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION pgroller_test.assert_not_null(description text, value anyelement)
RETURNS void AS $$
BEGIN
    IF value IS NULL THEN
        RAISE EXCEPTION '[test-assertion] %: expected NOT NULL', description;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION pgroller_test.assert_gt(description text, actual numeric, threshold numeric)
RETURNS void AS $$
BEGIN
    IF actual <= threshold THEN
        RAISE EXCEPTION '[test-assertion] %: expected % > %', description, actual, threshold;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION pgroller_test.assert_count(description text, table_name text, expected bigint)
RETURNS void AS $$
DECLARE
    actual bigint;
BEGIN
    EXECUTE format('SELECT count(*) FROM %I', table_name) INTO actual;
    IF actual != expected THEN
        RAISE EXCEPTION '[test-assertion] %: row count is % expected %', description, actual, expected;
    END IF;
END;
$$ LANGUAGE plpgsql;
"#;

/// Overall test report for all migrations.
#[derive(Debug)]
pub struct TestReport {
    pub results: Vec<MigrationTestResult>,
}

/// Result for a single migration test.
#[derive(Debug)]
pub struct MigrationTestResult {
    pub migration: Migration,
    pub status: TestStatus,
}

/// Status of a migration test.
#[derive(Debug)]
pub enum TestStatus {
    /// Round-trip was clean, or all diffs were annotated.
    Pass { covered_count: usize },
    /// There are uncovered diffs (test failure).
    Fail { uncovered: Vec<UncoveredDiff> },
    /// An error occurred during testing.
    Error(String),
    /// Stale annotations that don't match any diff.
    Warning {
        message: String,
        stale_schema: Vec<NoSchemaRollback>,
        stale_data: Vec<NoDataRollback>,
    },
}

impl TestReport {
    /// Returns true if all results are Pass or Warning (no Fail or Error).
    pub fn is_success(&self) -> bool {
        self.results.iter().all(|r| {
            matches!(
                r.status,
                TestStatus::Pass { .. } | TestStatus::Warning { .. }
            )
        })
    }
}

/// Run the full test suite for all discovered migrations.
pub async fn run_test(config: &PgrollerConfig) -> Result<TestReport> {
    let migrations_dir = Path::new(&config.migrations.dir);

    // Phase 1: Discovery
    ui::print_phase("Discovery");
    let migrations = discover_migrations(migrations_dir)?;
    ui::print_info("migrations", &format!("{} found", migrations.len()));
    ui::print_info("directory", &config.migrations.dir);
    ui::print_info(
        "postgres",
        &format!("v{}", config.database.postgres_version),
    );
    ui::print_info("reset", &config.test.reset_strategy);

    if !config.database.extensions.is_empty() {
        ui::print_info("extensions", &config.database.extensions.join(", "));
    }

    // Phase 2: Start single container
    ui::print_phase("Round-trip Testing");
    ui::print_subphase("starting postgres container");

    let container = PgContainer::start(&config.database.postgres_version).await?;
    let conn_str = container.connection_string();

    if !config.database.extensions.is_empty() {
        ui::print_subphase("installing extensions");
        container
            .install_extensions(&config.database.extensions)
            .await?;
    }

    println!();

    // Create a persistent connection for all tests
    let conn = PersistentConn::connect(&conn_str).await?;

    // Inject pgroller_test schema with assertion functions
    ui::print_subphase("injecting pgroller_test assertion functions");
    conn.execute(PGROLLER_TEST_SCHEMA).await?;

    // Skip baseline (version 0) — it's the foundation, not tested for round-trip
    let testable_migrations: Vec<_> = migrations
        .iter()
        .enumerate()
        .filter(|(_, m)| m.version > 0)
        .collect();

    let mut progress = TestProgress::new(testable_migrations.len());
    let mut results = Vec::new();
    let schema = &config.database.schema;
    let use_savepoint = config.test.reset_strategy == "savepoint";

    if use_savepoint {
        conn.execute(&format!("SET search_path TO \"{}\";", schema))
            .await?;
        conn.execute_file(&migrations[0].path.join("up.sql"))
            .await?;
    }

    for (idx, migration) in &testable_migrations {
        progress.start_migration(migration.version, &migration.description);

        // Prepare: get database to state N-1
        progress.step_detail(migration.version, &migration.description, "preparing state");
        prepare_state(&conn, &migrations, *idx, schema, use_savepoint).await?;

        // Test
        let status = run_single_test(config, migration, &conn).await;

        // Cleanup: restore for next iteration
        cleanup_state(&conn, migration, schema, use_savepoint).await?;

        match &status {
            TestStatus::Pass { covered_count } => {
                progress.finish_migration_pass(
                    migration.version,
                    &migration.description,
                    *covered_count,
                );
            }
            TestStatus::Fail { uncovered } => {
                progress.finish_migration_fail(
                    migration.version,
                    &migration.description,
                    uncovered.len(),
                );
            }
            TestStatus::Warning {
                stale_schema,
                stale_data,
                ..
            } => {
                progress.finish_migration_warning(
                    migration.version,
                    &migration.description,
                    stale_schema.len() + stale_data.len(),
                );
            }
            TestStatus::Error(msg) => {
                progress.finish_migration_error(migration.version, &migration.description, msg);
            }
        }

        results.push(MigrationTestResult {
            migration: (*migration).clone(),
            status,
        });

        // Always stop if up.sql or down.sql failed — production code is broken
        if let TestStatus::Error(ref msg) = results.last().unwrap().status {
            if msg.contains("[up.sql]") || msg.contains("[down.sql]") {
                break;
            }
        }

        if !config.test.continue_on_failure {
            if let TestStatus::Fail { .. } | TestStatus::Error(_) = &results.last().unwrap().status
            {
                break;
            }
        }
    }

    progress.finish_all();

    // Phase 3: Report details
    let has_issues = results.iter().any(|r| {
        matches!(
            r.status,
            TestStatus::Fail { .. } | TestStatus::Warning { .. } | TestStatus::Error(_)
        )
    });

    if has_issues {
        ui::print_separator();
        ui::print_phase("Details");

        for result in &results {
            match &result.status {
                TestStatus::Fail { uncovered } => {
                    let details: Vec<(String, String)> = uncovered
                        .iter()
                        .map(|u| (u.description.clone(), u.suggestion.clone()))
                        .collect();
                    ui::print_uncovered_details(
                        result.migration.version,
                        &result.migration.description,
                        &details,
                    );
                }
                TestStatus::Warning {
                    stale_schema,
                    stale_data,
                    ..
                } => {
                    let mut details: Vec<(String, String, String)> = stale_schema
                        .iter()
                        .map(|s| {
                            (
                                format!("@NoSchemaRollback({}={})", s.target.as_str(), s.name),
                                s.name.clone(),
                                s.reason.clone(),
                            )
                        })
                        .collect();
                    details.extend(stale_data.iter().map(|s| {
                        (
                            format!("@NoDataRollback(table={})", s.table),
                            s.table.clone(),
                            s.reason.clone(),
                        )
                    }));
                    ui::print_stale_details(
                        result.migration.version,
                        &result.migration.description,
                        &details,
                    );
                }
                TestStatus::Error(msg) => {
                    ui::print_error_details(
                        result.migration.version,
                        &result.migration.description,
                        msg,
                    );
                }
                _ => {}
            }
        }
    }

    // Summary
    let elapsed = progress.elapsed();
    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| matches!(r.status, TestStatus::Pass { .. }))
        .count();
    let warnings = results
        .iter()
        .filter(|r| matches!(r.status, TestStatus::Warning { .. }))
        .count();
    let production_errors = results
        .iter()
        .filter(|r| {
            if let TestStatus::Error(msg) = &r.status {
                msg.contains("[up.sql]") || msg.contains("[down.sql]")
            } else {
                false
            }
        })
        .count();
    let test_failures = results
        .iter()
        .filter(|r| {
            matches!(r.status, TestStatus::Fail { .. })
                || matches!(&r.status, TestStatus::Error(msg) if !msg.contains("[up.sql]") && !msg.contains("[down.sql]"))
        })
        .count();

    ui::print_separator();
    ui::print_summary(
        total,
        passed,
        warnings,
        test_failures,
        production_errors,
        elapsed,
    );

    Ok(TestReport { results })
}

/// Run a single migration test using the persistent connection.
async fn run_single_test(
    config: &PgrollerConfig,
    migration: &Migration,
    conn: &PersistentConn,
) -> TestStatus {
    match run_single_test_impl(config, migration, conn).await {
        Ok(status) => status,
        Err(e) => TestStatus::Error(e.to_string()),
    }
}

async fn run_single_test_impl(
    config: &PgrollerConfig,
    migration: &Migration,
    conn: &PersistentConn,
) -> Result<TestStatus> {
    let schema = &config.database.schema;

    // 1. Apply test-before-up.sql (seed data, optional)
    if migration.has_file("test-before-up.sql") {
        if let Err(e) = conn
            .execute_file(&migration.path.join("test-before-up.sql"))
            .await
        {
            return Ok(TestStatus::Error(format!("[test-before-up.sql] {}", e)));
        }
    }

    // 2. Snapshot A
    let schema_a = snapshot_schema_with_conn(conn, schema).await?;
    let seeded_tables = get_seeded_tables_with_conn(conn, schema).await?;
    let data_a = snapshot_data_with_conn(conn, &seeded_tables, schema).await?;

    // 3. Apply up.sql
    if let Err(e) = conn.execute_file(&migration.path.join("up.sql")).await {
        return Ok(TestStatus::Error(format!("[up.sql] {}", e)));
    }

    // 4. Execute test-after-up.sql (assertions, optional)
    if migration.has_file("test-after-up.sql") {
        if let Err(e) = conn
            .execute_file(&migration.path.join("test-after-up.sql"))
            .await
        {
            return Ok(TestStatus::Error(format!("[test-after-up.sql] {}", e)));
        }
    }

    // 5. Apply down.sql
    let down_content = std::fs::read_to_string(migration.path.join("down.sql"))?;
    let down_sql = filter_sql_only(&down_content);
    if !down_sql.trim().is_empty() {
        if let Err(e) = conn.execute(&down_sql).await {
            return Ok(TestStatus::Error(format!("[down.sql] {}", e)));
        }
    }

    // 6. Execute test-after-down.sql (rollback assertions, optional)
    if migration.has_file("test-after-down.sql") {
        if let Err(e) = conn
            .execute_file(&migration.path.join("test-after-down.sql"))
            .await
        {
            return Ok(TestStatus::Error(format!("[test-after-down.sql] {}", e)));
        }
    }

    // 7. Snapshot B
    let schema_b = snapshot_schema_with_conn(conn, schema).await?;
    let data_b = snapshot_data_with_conn(conn, &seeded_tables, schema).await?;

    // 8. Diff + match
    let mut schema_diff: SchemaDiff = diff_schemas(&schema_a, &schema_b);
    // Filter out OID-based constraint name differences (same constraint, different internal names)
    schema_diff
        .missing_constraints
        .retain(|c| !is_oid_constraint(&c.name));
    schema_diff
        .extra_constraints
        .retain(|c| !is_oid_constraint(&c.name));
    let data_diff: DataDiff = diff_data(&data_a, &data_b);
    let annotations = parse_annotations(&down_content)?;
    let match_result = match_diffs(&schema_diff, &data_diff, &annotations);

    // 9. Result
    if !match_result.uncovered.is_empty() {
        return Ok(TestStatus::Fail {
            uncovered: match_result.uncovered,
        });
    }

    if match_result.has_stale() {
        return Ok(TestStatus::Warning {
            message: format!("{} stale annotations", match_result.stale_count()),
            stale_schema: match_result.stale_schema,
            stale_data: match_result.stale_data,
        });
    }

    Ok(TestStatus::Pass {
        covered_count: match_result.covered.len(),
    })
}

/// Snapshot schema using a persistent connection.
pub async fn snapshot_schema_with_conn(
    conn: &PersistentConn,
    schema: &str,
) -> Result<crate::differ::SchemaSnapshot> {
    use crate::differ::{ColumnInfo, ConstraintInfo, SchemaSnapshot, TriggerInfo};

    let mut snapshot = SchemaSnapshot::default();

    let rows = conn.query(&format!(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = '{}' AND table_type = 'BASE TABLE' ORDER BY table_name", schema
    )).await?;
    snapshot.tables = rows.into_iter().map(|r| r[0].clone()).collect();

    let rows = conn.query(&format!(
        "SELECT table_name, column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = '{}' ORDER BY table_name, ordinal_position", schema
    )).await?;
    snapshot.columns = rows
        .into_iter()
        .map(|r| ColumnInfo {
            table: r[0].clone(),
            column: r[1].clone(),
            data_type: r[2].clone(),
            is_nullable: r[3] == "YES",
            default: if r[4] == "NULL" {
                None
            } else {
                Some(r[4].clone())
            },
        })
        .collect();

    let rows = conn
        .query(&format!(
            "SELECT indexname FROM pg_indexes WHERE schemaname = '{}' ORDER BY indexname",
            schema
        ))
        .await?;
    snapshot.indexes = rows.into_iter().map(|r| r[0].clone()).collect();

    let rows = conn.query(&format!(
        "SELECT tc.table_name, tc.constraint_name FROM information_schema.table_constraints tc WHERE tc.table_schema = '{}' ORDER BY tc.table_name, tc.constraint_name", schema
    )).await?;
    snapshot.constraints = rows
        .into_iter()
        .map(|r| ConstraintInfo {
            table: r[0].clone(),
            name: r[1].clone(),
        })
        .collect();

    let rows = conn.query(&format!(
        "SELECT routine_name FROM information_schema.routines WHERE routine_schema = '{}' ORDER BY routine_name", schema
    )).await?;
    snapshot.functions = rows.into_iter().map(|r| r[0].clone()).collect();

    let rows = conn.query(&format!(
        "SELECT sequence_name FROM information_schema.sequences WHERE sequence_schema = '{}' ORDER BY sequence_name", schema
    )).await?;
    snapshot.sequences = rows.into_iter().map(|r| r[0].clone()).collect();

    let rows = conn.query(&format!(
        "SELECT event_object_table, trigger_name FROM information_schema.triggers WHERE trigger_schema = '{}' ORDER BY event_object_table, trigger_name", schema
    )).await?;
    snapshot.triggers = rows
        .into_iter()
        .map(|r| TriggerInfo {
            table: r[0].clone(),
            name: r[1].clone(),
        })
        .collect();

    let rows = conn.query(&format!(
        "SELECT t.typname FROM pg_type t JOIN pg_namespace n ON t.typnamespace = n.oid WHERE n.nspname = '{}' AND t.typtype IN ('e', 'c') ORDER BY t.typname", schema
    )).await?;
    snapshot.types = rows.into_iter().map(|r| r[0].clone()).collect();

    Ok(snapshot)
}

/// Snapshot data using a persistent connection.
pub async fn snapshot_data_with_conn(
    conn: &PersistentConn,
    tables: &[String],
    schema: &str,
) -> Result<crate::differ::DataSnapshot> {
    use crate::differ::{DataSnapshot, TableSnapshot};

    let mut snapshot = DataSnapshot::default();

    for table in tables {
        let col_rows = conn.query(&format!(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = '{}' AND table_name = '{}' ORDER BY ordinal_position",
            schema, table
        )).await?;
        let columns: Vec<String> = col_rows.into_iter().map(|r| r[0].clone()).collect();

        if columns.is_empty() {
            continue;
        }

        let rows = conn
            .query(&format!(
                "SELECT * FROM \"{}\".\"{}\" ORDER BY \"{}\"",
                schema, table, columns[0]
            ))
            .await
            .unwrap_or_default();

        snapshot.tables.push(TableSnapshot {
            table: table.clone(),
            columns,
            rows,
        });
    }

    Ok(snapshot)
}

/// Get list of tables using persistent connection.
pub async fn get_seeded_tables_with_conn(
    conn: &PersistentConn,
    schema: &str,
) -> Result<Vec<String>> {
    let rows = conn.query(&format!(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = '{}' AND table_type = 'BASE TABLE' ORDER BY table_name", schema
    )).await?;
    Ok(rows.into_iter().map(|r| r[0].clone()).collect())
}

/// Filter out comment lines (starting with --) from SQL content,
/// returning only executable SQL.
/// Prepare database state to N-1 before testing migration N.
///
/// - `drop_schema`: DROP + CREATE schema, apply all prior up.sql from scratch. O(N²) total.
/// - `savepoint`: Schema already at N-1 from previous iteration, just BEGIN + SAVEPOINT. O(N) total.
async fn prepare_state(
    conn: &PersistentConn,
    migrations: &[Migration],
    idx: usize,
    schema: &str,
    use_savepoint: bool,
) -> Result<()> {
    if use_savepoint {
        conn.execute("BEGIN").await?;
        conn.execute("SAVEPOINT pgroller_test").await?;
    } else {
        let reset_sql = format!(
            "DROP SCHEMA IF EXISTS \"{}\" CASCADE; CREATE SCHEMA \"{}\"; SET search_path TO \"{}\";",
            schema, schema, schema
        );
        conn.execute(&reset_sql).await?;
        for prev in &migrations[..idx] {
            conn.execute_file(&prev.path.join("up.sql")).await?;
        }
    }
    Ok(())
}

/// Cleanup after a test and advance schema to state N for the next iteration.
///
/// - `drop_schema`: Nothing to do — next iteration rebuilds from scratch.
/// - `savepoint`: ROLLBACK to undo test, COMMIT, then apply up.sql to advance.
async fn cleanup_state(
    conn: &PersistentConn,
    migration: &Migration,
    _schema: &str,
    use_savepoint: bool,
) -> Result<()> {
    if use_savepoint {
        let _ = conn.execute("ROLLBACK TO SAVEPOINT pgroller_test").await;
        let _ = conn.execute("COMMIT").await;
        conn.execute_file(&migration.path.join("up.sql")).await?;
    }
    Ok(())
}

/// Print a migration with its rollback annotations as warnings.
fn print_migration_with_annotations(migration: &Migration) {
    let down_path = migration.path.join("down.sql");
    let annotations = if down_path.exists() {
        let content = std::fs::read_to_string(&down_path).unwrap_or_default();
        parse_annotations(&content).unwrap_or_default()
    } else {
        crate::parser::Annotations::default()
    };

    let has_warnings = !annotations.schema.is_empty() || !annotations.data.is_empty();

    if has_warnings {
        println!(
            "    {} {}__{}",
            "→".cyan(),
            migration.version,
            migration.description
        );
        for ann in &annotations.schema {
            println!(
                "      {} @NoSchemaRollback({}={}) — {}",
                "⚠".yellow(),
                ann.target.as_str(),
                ann.name,
                ann.reason
            );
        }
        for ann in &annotations.data {
            println!(
                "      {} @NoDataRollback(table={}) — {}",
                "⚠".yellow(),
                ann.table,
                ann.reason
            );
        }
    } else {
        println!(
            "    {} {}__{}",
            "→".cyan(),
            migration.version,
            migration.description
        );
    }
}

pub fn filter_sql_only(content: &str) -> String {
    content
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ═══════════════════════════════════════════════════════════════════════════════
// Status, Migrate, Rollback Commands
// ═══════════════════════════════════════════════════════════════════════════════

/// Advisory lock ID for preventing concurrent migrations.
const ADVISORY_LOCK_ID: i64 = 748370677;

/// Ensure the pgroller_history table exists in the target database.
async fn ensure_history_table(conn: &PersistentConn) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pgroller_history (
            version BIGINT PRIMARY KEY,
            description VARCHAR(255) NOT NULL,
            applied_at TIMESTAMP NOT NULL DEFAULT NOW(),
            checksum VARCHAR(64) NOT NULL,
            success BOOLEAN NOT NULL DEFAULT TRUE
        )",
    )
    .await
}

/// Get list of applied migrations from the history table.
/// Returns (version, description, checksum) ordered by version.
async fn get_applied_migrations(conn: &PersistentConn) -> Result<Vec<(u64, String, String)>> {
    let rows = conn.query(
        "SELECT version, description, checksum FROM pgroller_history WHERE success = TRUE ORDER BY version"
    ).await?;

    let mut result = Vec::new();
    for row in rows {
        let version: u64 = row[0].parse().unwrap_or(0);
        let description = row[1].clone();
        let checksum = row[2].clone();
        result.push((version, description, checksum));
    }
    Ok(result)
}

/// Compute a checksum for a file using DefaultHasher from std.
fn compute_checksum(path: &Path) -> Result<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let content = std::fs::read_to_string(path)?;
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

/// Show migration status against a database.
pub async fn run_status(config: &PgrollerConfig, database_url: &str) -> Result<()> {
    let migrations_dir = Path::new(&config.migrations.dir);

    ui::print_phase("Migration Status");

    // Connect
    ui::print_subphase("connecting to database");
    let conn = PersistentConn::connect(database_url).await?;

    // Ensure history table
    ensure_history_table(&conn).await?;

    // Get applied migrations
    let applied = get_applied_migrations(&conn).await?;

    // Discover migrations from disk
    let disk_migrations = discover_migrations(migrations_dir)?;

    // Build status
    ui::print_info("database", database_url);
    ui::print_info("migrations dir", &config.migrations.dir);
    println!();

    let mut pending_count = 0;
    let mut applied_count = 0;
    let mut mismatch_count = 0;

    for migration in &disk_migrations {
        let applied_entry = applied.iter().find(|(v, _, _)| *v == migration.version);

        match applied_entry {
            Some((_v, _desc, checksum)) => {
                // Check checksum
                let disk_checksum = compute_checksum(&migration.path.join("up.sql"))?;
                if *checksum != disk_checksum {
                    println!(
                        "    {} {} {}__{} — {}",
                        "✗".red().bold(),
                        "applied".red(),
                        migration.version,
                        migration.description,
                        "CHECKSUM MISMATCH".red().bold()
                    );
                    mismatch_count += 1;
                } else {
                    println!(
                        "    {} {} {}__{}",
                        "✓".green().bold(),
                        "applied".green(),
                        migration.version,
                        migration.description,
                    );
                }
                applied_count += 1;
            }
            None => {
                println!(
                    "    {} {} {}__{}",
                    "○".cyan().bold(),
                    "pending".cyan(),
                    migration.version,
                    migration.description,
                );
                pending_count += 1;
            }
        }
    }

    // Check for applied migrations not on disk
    for (version, description, _) in &applied {
        let on_disk = disk_migrations.iter().any(|m| m.version == *version);
        if !on_disk {
            println!(
                "    {} {} {}__{} — {}",
                "?".yellow().bold(),
                "orphan".yellow(),
                version,
                description,
                "not found on disk".yellow()
            );
        }
    }

    println!();
    ui::print_info("applied", &format!("{}", applied_count));
    ui::print_info("pending", &format!("{}", pending_count));
    if mismatch_count > 0 {
        ui::print_info("mismatches", &format!("{}", mismatch_count));
    }

    Ok(())
}

/// Apply pending migrations to a database.
pub async fn run_migrate(
    config: &PgrollerConfig,
    database_url: &str,
    dry_run: bool,
    accept: bool,
) -> Result<()> {
    let migrations_dir = Path::new(&config.migrations.dir);

    ui::print_phase("Migrate");

    // Connect
    ui::print_subphase("connecting to database");
    let conn = PersistentConn::connect(database_url).await?;

    // Ensure history table
    ensure_history_table(&conn).await?;

    // Advisory lock
    ui::print_subphase("acquiring advisory lock");
    conn.execute(&format!("SELECT pg_advisory_lock({})", ADVISORY_LOCK_ID))
        .await?;

    // Get applied migrations
    let applied = get_applied_migrations(&conn).await?;
    let max_applied = applied.iter().map(|(v, _, _)| *v).max().unwrap_or(0);

    // Discover migrations from disk
    let disk_migrations = discover_migrations(migrations_dir)?;

    // Find pending migrations
    // Include baseline (version 0) if nothing has been applied yet
    let pending: Vec<&Migration> = disk_migrations
        .iter()
        .filter(|m| {
            if applied.is_empty() {
                // Fresh DB: apply everything including baseline
                true
            } else {
                // Existing DB: skip baseline, only apply versions > max applied
                m.version > 0 && m.version > max_applied
            }
        })
        .collect();

    // Verify checksums of already-applied migrations
    for (version, _desc, checksum) in &applied {
        if let Some(disk_m) = disk_migrations.iter().find(|m| m.version == *version) {
            let disk_checksum = compute_checksum(&disk_m.path.join("up.sql"))?;
            if *checksum != disk_checksum {
                // Release lock before failing
                let _ = conn
                    .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                    .await;
                return Err(PgrollerError::Execution(format!(
                    "Checksum mismatch for migration {}__{}: applied='{}' disk='{}'.\n  \
                     Refusing to migrate — resolve the mismatch first.",
                    version, _desc, checksum, disk_checksum
                )));
            }
        }
    }

    ui::print_info("applied", &format!("{}", applied.len()));
    ui::print_info("pending", &format!("{}", pending.len()));

    if pending.is_empty() {
        ui::print_subphase("nothing to migrate");
        let _ = conn
            .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
            .await;
        return Ok(());
    }

    if dry_run {
        ui::print_phase("Dry Run — no changes will be made");
        println!();
        for m in &pending {
            print_migration_with_annotations(m);
        }
        println!();
        ui::print_subphase("run without --dry-run to apply");
        let _ = conn
            .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
            .await;
        return Ok(());
    }

    // Show plan and ask for confirmation
    println!();
    for m in &pending {
        print_migration_with_annotations(m);
    }
    println!();

    if !accept {
        use std::io::{self, Write};
        print!("    Apply {} migration(s)? [y/N]: ", pending.len());
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let answer = input.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            ui::print_subphase("aborted");
            let _ = conn
                .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                .await;
            return Ok(());
        }
    }

    // Apply each pending migration
    ui::print_phase("Applying Migrations");
    let mut applied_count = 0;

    for migration in &pending {
        ui::print_subphase(&format!(
            "applying {}__{}...",
            migration.version, migration.description
        ));

        let checksum = compute_checksum(&migration.path.join("up.sql"))?;

        // BEGIN transaction
        conn.execute("BEGIN").await?;

        // Execute up.sql
        match conn.execute_file(&migration.path.join("up.sql")).await {
            Ok(_) => {}
            Err(e) => {
                let _ = conn.execute("ROLLBACK").await;
                let _ = conn
                    .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                    .await;
                eprintln!(
                    "\n    {} Failed to apply {}__{}: {}",
                    "✗".red().bold(),
                    migration.version,
                    migration.description,
                    e
                );
                return Err(e);
            }
        }

        // Insert into history
        let insert_sql = format!(
            "INSERT INTO pgroller_history (version, description, checksum, success) VALUES ({}, '{}', '{}', TRUE)",
            migration.version,
            migration.description.replace('\'', "''"),
            checksum
        );
        match conn.execute(&insert_sql).await {
            Ok(_) => {}
            Err(e) => {
                let _ = conn.execute("ROLLBACK").await;
                let _ = conn
                    .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                    .await;
                return Err(e);
            }
        }

        // COMMIT
        conn.execute("COMMIT").await?;

        println!(
            "    {} {}__{}",
            "✓".green().bold(),
            migration.version,
            migration.description
        );
        applied_count += 1;
    }

    // Release advisory lock
    let _ = conn
        .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
        .await;

    println!();
    ui::print_info("applied", &format!("{} migration(s)", applied_count));

    Ok(())
}

/// Rollback applied migrations from a database.
pub async fn run_rollback(
    config: &PgrollerConfig,
    database_url: &str,
    steps: usize,
    dry_run: bool,
    accept: bool,
) -> Result<()> {
    let migrations_dir = Path::new(&config.migrations.dir);

    ui::print_phase("Rollback");

    // Connect
    ui::print_subphase("connecting to database");
    let conn = PersistentConn::connect(database_url).await?;

    // Ensure history table
    ensure_history_table(&conn).await?;

    // Advisory lock
    ui::print_subphase("acquiring advisory lock");
    conn.execute(&format!("SELECT pg_advisory_lock({})", ADVISORY_LOCK_ID))
        .await?;

    // Get applied migrations
    let applied = get_applied_migrations(&conn).await?;

    if applied.is_empty() {
        ui::print_subphase("no migrations to rollback");
        let _ = conn
            .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
            .await;
        return Ok(());
    }

    // Discover migrations from disk
    let disk_migrations = discover_migrations(migrations_dir)?;

    // Take last N applied migrations (in reverse order)
    let to_rollback: Vec<&(u64, String, String)> = applied.iter().rev().take(steps).collect();

    ui::print_info(
        "rolling back",
        &format!("{} migration(s)", to_rollback.len()),
    );

    if dry_run {
        ui::print_phase("Dry Run — no changes will be made");
        println!();
        for (version, description, _) in &to_rollback {
            // Check for annotations in down.sql
            if let Some(disk_m) = disk_migrations.iter().find(|m| m.version == *version) {
                let down_path = disk_m.path.join("down.sql");
                if down_path.exists() {
                    let content = std::fs::read_to_string(&down_path)?;
                    let annotations = parse_annotations(&content)?;
                    let mut warnings = Vec::new();
                    for ann in &annotations.schema {
                        warnings.push(format!(
                            "@NoSchemaRollback({}={})",
                            ann.target.as_str(),
                            ann.name
                        ));
                    }
                    for ann in &annotations.data {
                        warnings.push(format!("@NoDataRollback(table={})", ann.table));
                    }
                    if warnings.is_empty() {
                        println!("    {} {}__{}", "←".cyan(), version, description);
                    } else {
                        println!(
                            "    {} {}__{} — {}",
                            "⚠".yellow(),
                            version,
                            description,
                            "has rollback annotations:".yellow()
                        );
                        for w in &warnings {
                            println!("      {} {}", "│".yellow(), w.yellow());
                        }
                    }
                } else {
                    println!(
                        "    {} {}__{} — {}",
                        "✗".red(),
                        version,
                        description,
                        "down.sql not found on disk!".red()
                    );
                }
            } else {
                println!(
                    "    {} {}__{} — {}",
                    "✗".red(),
                    version,
                    description,
                    "migration not found on disk!".red()
                );
            }
        }
        println!();
        ui::print_subphase("run without --dry-run to apply");
        let _ = conn
            .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
            .await;
        return Ok(());
    }

    // Show plan and ask for confirmation
    println!();
    for (version, description, _) in &to_rollback {
        if let Some(disk_m) = disk_migrations.iter().find(|m| m.version == *version) {
            print_migration_with_annotations(disk_m);
        } else {
            println!("    {} {}__{}", "←".cyan(), version, description);
        }
    }
    println!();

    if !accept {
        use std::io::{self, Write};
        print!("    Rollback {} migration(s)? [y/N]: ", to_rollback.len());
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let answer = input.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            ui::print_subphase("aborted");
            let _ = conn
                .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                .await;
            return Ok(());
        }
    }

    // Apply rollbacks
    ui::print_phase("Rolling Back");
    let mut rolled_back_count = 0;

    for (version, description, _) in &to_rollback {
        ui::print_subphase(&format!("rolling back {}__{}...", version, description));

        // Find matching disk migration
        let disk_m = match disk_migrations.iter().find(|m| m.version == *version) {
            Some(m) => m,
            None => {
                let _ = conn
                    .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                    .await;
                return Err(PgrollerError::Execution(format!(
                    "Cannot rollback {}__{}: migration not found on disk",
                    version, description
                )));
            }
        };

        let down_path = disk_m.path.join("down.sql");
        if !down_path.exists() {
            let _ = conn
                .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                .await;
            return Err(PgrollerError::Execution(format!(
                "Cannot rollback {}__{}: down.sql not found",
                version, description
            )));
        }

        // Read and filter SQL (strip annotation comments)
        let content = std::fs::read_to_string(&down_path)?;
        let sql = filter_sql_only(&content);

        // BEGIN transaction
        conn.execute("BEGIN").await?;

        // Execute the down.sql
        if !sql.trim().is_empty() {
            match conn.execute(&sql).await {
                Ok(_) => {}
                Err(e) => {
                    let _ = conn.execute("ROLLBACK").await;
                    let _ = conn
                        .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                        .await;
                    eprintln!(
                        "\n    {} Failed to rollback {}__{}: {}",
                        "✗".red().bold(),
                        version,
                        description,
                        e
                    );
                    return Err(e);
                }
            }
        }

        // Delete from history
        let delete_sql = format!("DELETE FROM pgroller_history WHERE version = {}", version);
        match conn.execute(&delete_sql).await {
            Ok(_) => {}
            Err(e) => {
                let _ = conn.execute("ROLLBACK").await;
                let _ = conn
                    .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
                    .await;
                return Err(e);
            }
        }

        // COMMIT
        conn.execute("COMMIT").await?;

        println!("    {} {}__{}", "✓".green().bold(), version, description);
        rolled_back_count += 1;
    }

    // Release advisory lock
    let _ = conn
        .execute(&format!("SELECT pg_advisory_unlock({})", ADVISORY_LOCK_ID))
        .await;

    println!();
    ui::print_info(
        "rolled back",
        &format!("{} migration(s)", rolled_back_count),
    );

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// New Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Create a new migration folder with auto-incremented version.
pub fn run_new(config: &PgrollerConfig, name: &str, accept: bool) -> Result<()> {
    let migrations_dir = Path::new(&config.migrations.dir);

    ui::print_phase("New Migration");

    // Normalize name: lowercase, replace spaces/hyphens with underscores, strip non-alphanumeric
    let normalized: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");

    if normalized.is_empty() {
        return Err(PgrollerError::Config(
            "Migration name cannot be empty".to_string(),
        ));
    }

    // Find next version number
    let next_version = find_next_version(migrations_dir)?;

    let folder_name = format!("{}__{}", next_version, normalized);
    let folder_path = migrations_dir.join(&folder_name);

    // Show plan
    println!();
    println!("    Will create:");
    println!("      {}/", folder_name);
    println!("      ├── up.sql");
    println!("      ├── down.sql");
    println!("      ├── test-before-up.sql");
    println!("      ├── test-after-up.sql");
    println!("      └── test-after-down.sql");
    println!();

    // Confirm
    if !accept {
        use std::io::{self, Write};
        print!("    Proceed? [Y/n]: ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let answer = input.trim().to_lowercase();
        if answer == "n" || answer == "no" {
            ui::print_subphase("aborted");
            return Ok(());
        }
    }

    // Create
    std::fs::create_dir_all(&folder_path)?;

    std::fs::write(
        folder_path.join("up.sql"),
        "-- Migration: add your SQL here\n",
    )?;

    std::fs::write(
        folder_path.join("down.sql"),
        "-- Rollback: reverse the migration above\n",
    )?;

    std::fs::write(
        folder_path.join("test-before-up.sql"),
        "-- Seed: insert test data before migration runs\n",
    )?;

    std::fs::write(
        folder_path.join("test-after-up.sql"),
        "-- Assertions: verify migration worked (runs after up.sql)\n",
    )?;

    std::fs::write(
        folder_path.join("test-after-down.sql"),
        "-- Assertions: verify rollback worked (runs after down.sql)\n",
    )?;

    println!("    {} Created {}/", "✓".green().bold(), folder_name);
    println!();

    Ok(())
}

/// Find the next available version number by scanning existing migration folders.
fn find_next_version(dir: &Path) -> Result<u64> {
    if !dir.exists() {
        return Ok(1);
    }

    let pattern = regex::Regex::new(r"^(\d+)__")?;
    let mut max_version: u64 = 0;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(caps) = pattern.captures(&name) {
            if let Ok(v) = caps[1].parse::<u64>() {
                if v > max_version {
                    max_version = v;
                }
            }
        }
    }

    Ok(max_version + 1)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Verify Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Verify a database schema matches the expected migration state.
/// Spins up a reference Testcontainer, applies all migrations up to the target's
/// current version, then compares schemas.
pub async fn run_verify(config: &PgrollerConfig, database_url: &str) -> Result<()> {
    let migrations_dir = Path::new(&config.migrations.dir);

    ui::print_phase("Verify");

    // Connect to target
    ui::print_subphase("connecting to target database");
    let target_conn = PersistentConn::connect(database_url).await?;

    // Ensure history table exists and get current state
    ensure_history_table(&target_conn).await?;
    let applied = get_applied_migrations(&target_conn).await?;
    let max_applied = applied.iter().map(|(v, _, _)| *v).max().unwrap_or(0);

    ui::print_info("target version", &format!("{}", max_applied));

    // Discover disk migrations
    let disk_migrations = discover_migrations(migrations_dir)?;

    // Spin up reference container
    ui::print_subphase("starting reference container");
    let container = PgContainer::start(&config.database.postgres_version).await?;
    let ref_conn_str = container.connection_string();
    let ref_conn = PersistentConn::connect(&ref_conn_str).await?;

    // Apply all migrations 0..max_applied to reference
    ui::print_subphase(&format!(
        "applying migrations 0..{} to reference",
        max_applied
    ));
    for m in &disk_migrations {
        if m.version > max_applied {
            break;
        }
        ref_conn.execute_file(&m.path.join("up.sql")).await?;
    }

    // Snapshot both schemas (exclude pgroller_history — it's infrastructure, not app schema)
    ui::print_subphase("comparing schemas");
    let schema = &config.database.schema;
    let target_schema = snapshot_schema_with_conn(&target_conn, schema).await?;
    let ref_schema = snapshot_schema_with_conn(&ref_conn, schema).await?;

    // Filter out pgroller_history and OID-based constraint name differences
    let diff = diff_schemas(&target_schema, &ref_schema);
    let diff = filter_verify_diff(diff);

    if diff.is_empty() {
        ui::print_phase("Result");
        println!(
            "    {} Schema matches expected state (version {})",
            "✓".green().bold(),
            max_applied
        );
        println!();
        Ok(())
    } else {
        ui::print_phase("Result");
        println!("    {} Schema drift detected:\n", "✗".red().bold());

        for col in &diff.extra_columns {
            println!(
                "      {} Extra column: {}.{} (in target, not in reference)",
                "╭─".red(),
                col.table,
                col.column
            );
        }
        for col in &diff.missing_columns {
            println!(
                "      {} Missing column: {}.{} (in reference, not in target)",
                "╭─".red(),
                col.table,
                col.column
            );
        }
        for col in &diff.modified_columns {
            println!(
                "      {} Modified column: {}.{} (type/default/nullable differs)",
                "╭─".yellow(),
                col.table,
                col.column
            );
        }
        for t in &diff.extra_tables {
            println!(
                "      {} Extra table: {} (in target, not in reference)",
                "╭─".red(),
                t
            );
        }
        for t in &diff.missing_tables {
            println!(
                "      {} Missing table: {} (in reference, not in target)",
                "╭─".red(),
                t
            );
        }
        for i in &diff.extra_indexes {
            println!(
                "      {} Extra index: {} (in target, not in reference)",
                "╭─".red(),
                i
            );
        }
        for i in &diff.missing_indexes {
            println!(
                "      {} Missing index: {} (in reference, not in target)",
                "╭─".red(),
                i
            );
        }
        for c in &diff.extra_constraints {
            println!(
                "      {} Extra constraint: {}.{} (in target, not in reference)",
                "╭─".red(),
                c.table,
                c.name
            );
        }
        for c in &diff.missing_constraints {
            println!(
                "      {} Missing constraint: {}.{} (in reference, not in target)",
                "╭─".red(),
                c.table,
                c.name
            );
        }
        for s in &diff.extra_sequences {
            println!(
                "      {} Extra sequence: {} (in target, not in reference)",
                "╭─".red(),
                s
            );
        }
        for s in &diff.missing_sequences {
            println!(
                "      {} Missing sequence: {} (in reference, not in target)",
                "╭─".red(),
                s
            );
        }
        for t in &diff.extra_types {
            println!(
                "      {} Extra type: {} (in target, not in reference)",
                "╭─".red(),
                t
            );
        }
        for t in &diff.missing_types {
            println!(
                "      {} Missing type: {} (in reference, not in target)",
                "╭─".red(),
                t
            );
        }
        for f in &diff.extra_functions {
            println!(
                "      {} Extra function: {} (in target, not in reference)",
                "╭─".red(),
                f
            );
        }
        for f in &diff.missing_functions {
            println!(
                "      {} Missing function: {} (in reference, not in target)",
                "╭─".red(),
                f
            );
        }
        for t in &diff.extra_triggers {
            println!(
                "      {} Extra trigger: {}.{} (in target, not in reference)",
                "╭─".red(),
                t.table,
                t.name
            );
        }
        for t in &diff.missing_triggers {
            println!(
                "      {} Missing trigger: {}.{} (in reference, not in target)",
                "╭─".red(),
                t.table,
                t.name
            );
        }
        println!();

        Err(PgrollerError::Execution(
            "Schema verification failed: drift detected".to_string(),
        ))
    }
}

/// Filter out noise from verify diff:
/// - pgroller_history table (infrastructure, not app schema)
/// - OID-based constraint names (differ between instances but are semantically identical)
fn filter_verify_diff(mut diff: SchemaDiff) -> SchemaDiff {
    // Remove pgroller_history references
    diff.missing_tables.retain(|t| t != "pgroller_history");
    diff.extra_tables.retain(|t| t != "pgroller_history");
    diff.missing_columns
        .retain(|c| c.table != "pgroller_history");
    diff.extra_columns.retain(|c| c.table != "pgroller_history");
    diff.missing_indexes
        .retain(|i| !i.contains("pgroller_history"));
    diff.extra_indexes
        .retain(|i| !i.contains("pgroller_history"));
    diff.missing_constraints
        .retain(|c| c.table != "pgroller_history");
    diff.extra_constraints
        .retain(|c| c.table != "pgroller_history");
    diff.missing_types.retain(|t| t != "pgroller_history");
    diff.extra_types.retain(|t| t != "pgroller_history");
    diff.missing_sequences
        .retain(|s| !s.contains("pgroller_history"));
    diff.extra_sequences
        .retain(|s| !s.contains("pgroller_history"));

    // Remove OID-based NOT NULL constraint differences
    // These look like: 2200_16384_1_not_null — they're the same constraint, just different OIDs
    diff.missing_constraints
        .retain(|c| !is_oid_constraint(&c.name));
    diff.extra_constraints
        .retain(|c| !is_oid_constraint(&c.name));

    diff
}

/// Check if a constraint name is an OID-based auto-generated name (e.g., "2200_16384_1_not_null")
pub fn is_oid_constraint(name: &str) -> bool {
    // OID constraints match pattern: digits_digits_digits_not_null
    let parts: Vec<&str> = name.split('_').collect();
    if parts.len() >= 4 && parts.last() == Some(&"null") {
        return parts[..parts.len() - 1]
            .iter()
            .all(|p| *p == "not" || p.chars().all(|c| c.is_ascii_digit()));
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════════════
// Baseline Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Collapse all existing migrations into a new baseline.
///
/// 1. Spins up a fresh Postgres
/// 2. Applies all migrations in order (up.sql only)
/// 3. Dumps the final schema as the new 0__baseline/up.sql
/// 4. Generates an empty test.sql
/// 5. Moves old migrations to archive/
pub async fn run_baseline(config: &PgrollerConfig, dry_run: bool) -> Result<()> {
    let migrations_dir = Path::new(&config.migrations.dir);

    ui::print_phase("Baseline Generation");

    // Discover existing migrations (relaxed — don't require 0__baseline to exist)
    let migrations = discover_migrations_for_baseline(migrations_dir)?;
    ui::print_info("migrations found", &format!("{}", migrations.len()));

    if migrations.is_empty() {
        ui::print_info("status", "no migrations to collapse");
        return Ok(());
    }

    // Start fresh Postgres
    ui::print_subphase("starting postgres container");
    let container = PgContainer::start(&config.database.postgres_version).await?;
    let conn_str = container.connection_string();

    // Install extensions
    if !config.database.extensions.is_empty() {
        ui::print_subphase("installing extensions");
        container
            .install_extensions(&config.database.extensions)
            .await?;
    }

    // Apply all migrations in order
    ui::print_subphase(&format!("applying {} migrations", migrations.len()));
    for m in &migrations {
        execute_file(&conn_str, &m.path.join("up.sql")).await?;
    }

    // Dump schema
    ui::print_subphase("dumping final schema");
    let schema_sql = dump_schema(&conn_str, &config.database.schema).await?;

    // Generate baseline files
    let baseline_dir = migrations_dir.join("0__baseline");
    let archive_dir = migrations_dir.join("archive");

    if dry_run {
        ui::print_phase("Dry Run — no changes made");
        println!();
        println!(
            "    Would create: {}/up.sql ({} bytes)",
            baseline_dir.display(),
            schema_sql.len()
        );
        println!("    Would create: {}/test.sql", baseline_dir.display());
        println!(
            "    Would move {} migrations to: {}/",
            migrations.len(),
            archive_dir.display()
        );
        println!();
        for m in &migrations {
            println!(
                "      {} {}__{}/",
                console::style("→").dim(),
                m.version,
                m.description
            );
        }
        println!();
        ui::print_subphase("run without --dry-run to apply");
        return Ok(());
    }

    // Create archive directory
    std::fs::create_dir_all(&archive_dir)?;

    // Move existing migrations to archive
    ui::print_subphase("archiving old migrations");
    for m in &migrations {
        let dest = archive_dir.join(m.path.file_name().unwrap());
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        std::fs::rename(&m.path, &dest)?;
    }

    // Create new baseline (directory may have been moved to archive, recreate)
    ui::print_subphase("writing new baseline");
    std::fs::create_dir_all(&baseline_dir)?;

    std::fs::write(baseline_dir.join("up.sql"), &schema_sql)?;

    std::fs::write(
        baseline_dir.join("test.sql"),
        "-- Add seed data and assertions for the baseline schema.\n",
    )?;

    ui::print_phase("Done");
    ui::print_info("baseline", &format!("{}/up.sql", baseline_dir.display()));
    ui::print_info(
        "archived",
        &format!(
            "{} migrations → {}/",
            migrations.len(),
            archive_dir.display()
        ),
    );
    ui::print_subphase("review the generated up.sql and add seed data to test.sql");

    Ok(())
}

/// Discover migrations without requiring 0__baseline to exist.
/// Used by the baseline command to find all existing migrations to collapse.
fn discover_migrations_for_baseline(dir: &Path) -> Result<Vec<Migration>> {
    use regex::Regex;
    use std::collections::HashMap;

    if !dir.exists() {
        return Err(PgrollerError::Discovery(format!(
            "Migrations directory does not exist: {}",
            dir.display()
        )));
    }

    let pattern = Regex::new(r"^(\d+)__([a-z][a-z0-9_]*)$")?;
    let mut migrations: Vec<Migration> = Vec::new();
    let mut versions_seen: HashMap<u64, String> = HashMap::new();

    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name != "archive"
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let entry_path = entry.path();

        let caps = match pattern.captures(&dir_name) {
            Some(c) => c,
            None => continue, // skip non-migration directories
        };

        let version_str = &caps[1];
        let description = caps[2].to_string();

        if version_str.len() > 1 && version_str.starts_with('0') {
            continue;
        }

        let version: u64 = match version_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        if versions_seen.contains_key(&version) {
            continue;
        }
        versions_seen.insert(version, dir_name.clone());

        // Only require up.sql for baseline generation
        if !entry_path.join("up.sql").exists() {
            continue;
        }

        migrations.push(Migration {
            version,
            description,
            path: entry_path,
        });
    }

    migrations.sort_by_key(|m| m.version);
    Ok(migrations)
}

/// Dump the current database schema as SQL using information_schema queries.
/// Produces CREATE TABLE, CREATE INDEX, etc. statements.
async fn dump_schema(conn_str: &str, schema: &str) -> Result<String> {
    let mut sql = String::new();
    sql.push_str("-- Generated by pgroller baseline\n");
    sql.push_str("-- Do not edit manually — regenerate with: pgroller baseline\n\n");

    // Get all tables
    let tables_query = format!(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = '{}' AND table_type = 'BASE TABLE' \
         ORDER BY table_name",
        schema
    );
    let tables = execute_query(conn_str, &tables_query).await?;

    for table_row in &tables {
        let table_name = &table_row[0];

        // Get column definitions
        let cols_query = format!(
            "SELECT column_name, data_type, character_maximum_length, \
                    is_nullable, column_default, udt_name \
             FROM information_schema.columns \
             WHERE table_schema = '{}' AND table_name = '{}' \
             ORDER BY ordinal_position",
            schema, table_name
        );
        let columns = execute_query(conn_str, &cols_query).await?;

        sql.push_str(&format!("CREATE TABLE \"{}\" (\n", table_name));

        let mut col_defs = Vec::new();
        for col in &columns {
            let col_name = &col[0];
            let data_type = &col[1];
            let max_length = &col[2];
            let is_nullable = &col[3];
            let default = &col[4];
            let udt_name = &col[5];

            let type_str = format_column_type(data_type, max_length, udt_name);

            let mut def = format!("    \"{}\" {}", col_name, type_str);

            if is_nullable == "NO" {
                def.push_str(" NOT NULL");
            }

            if default != "NULL" && !default.is_empty() {
                def.push_str(&format!(" DEFAULT {}", default));
            }

            col_defs.push(def);
        }

        // Get primary key
        let pk_query = format!(
            "SELECT kcu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
             WHERE tc.table_schema = '{}' AND tc.table_name = '{}' \
               AND tc.constraint_type = 'PRIMARY KEY' \
             ORDER BY kcu.ordinal_position",
            schema, table_name
        );
        let pk_cols = execute_query(conn_str, &pk_query).await?;

        if !pk_cols.is_empty() {
            let pk_col_names: Vec<String> =
                pk_cols.iter().map(|r| format!("\"{}\"", r[0])).collect();
            col_defs.push(format!("    PRIMARY KEY ({})", pk_col_names.join(", ")));
        }

        // Get unique constraints (not primary key)
        let unique_query = format!(
            "SELECT tc.constraint_name, kcu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
             WHERE tc.table_schema = '{}' AND tc.table_name = '{}' \
               AND tc.constraint_type = 'UNIQUE' \
             ORDER BY tc.constraint_name, kcu.ordinal_position",
            schema, table_name
        );
        let unique_rows = execute_query(conn_str, &unique_query).await?;

        // Group by constraint name
        let mut unique_constraints: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for row in &unique_rows {
            unique_constraints
                .entry(row[0].clone())
                .or_default()
                .push(format!("\"{}\"", row[1]));
        }

        for (_name, cols) in &unique_constraints {
            col_defs.push(format!("    UNIQUE ({})", cols.join(", ")));
        }

        sql.push_str(&col_defs.join(",\n"));
        sql.push_str("\n);\n\n");
    }

    // Get indexes (non-primary, non-unique constraint)
    let idx_query = format!(
        "SELECT indexname, indexdef FROM pg_indexes \
         WHERE schemaname = '{}' \
         AND indexname NOT IN ( \
           SELECT constraint_name FROM information_schema.table_constraints \
           WHERE table_schema = '{}' \
         ) \
         ORDER BY indexname",
        schema, schema
    );
    let indexes = execute_query(conn_str, &idx_query).await?;

    if !indexes.is_empty() {
        sql.push_str("-- Indexes\n");
        for idx in &indexes {
            sql.push_str(&format!("{};\n", idx[1]));
        }
        sql.push('\n');
    }

    // Get sequences
    let seq_query = format!(
        "SELECT sequence_name, data_type, start_value, increment \
         FROM information_schema.sequences \
         WHERE sequence_schema = '{}' \
         ORDER BY sequence_name",
        schema
    );
    let sequences = execute_query(conn_str, &seq_query).await?;

    if !sequences.is_empty() {
        sql.push_str("-- Sequences\n");
        for seq in &sequences {
            sql.push_str(&format!("CREATE SEQUENCE \"{}\";\n", seq[0]));
        }
        sql.push('\n');
    }

    // Get custom types (enums)
    let types_query = format!(
        "SELECT t.typname, string_agg(e.enumlabel, ',' ORDER BY e.enumsortorder) \
         FROM pg_type t \
         JOIN pg_namespace n ON t.typnamespace = n.oid \
         LEFT JOIN pg_enum e ON t.oid = e.enumtypid \
         WHERE n.nspname = '{}' AND t.typtype = 'e' \
         GROUP BY t.typname \
         ORDER BY t.typname",
        schema
    );
    let types = execute_query(conn_str, &types_query).await?;

    if !types.is_empty() {
        sql.push_str("-- Types\n");
        for typ in &types {
            let labels: Vec<&str> = typ[1].split(',').collect();
            let quoted_labels: Vec<String> = labels.iter().map(|l| format!("'{}'", l)).collect();
            sql.push_str(&format!(
                "CREATE TYPE \"{}\" AS ENUM ({});\n",
                typ[0],
                quoted_labels.join(", ")
            ));
        }
        sql.push('\n');
    }

    Ok(sql)
}

fn format_column_type(data_type: &str, max_length: &str, udt_name: &str) -> String {
    match data_type {
        "character varying" => {
            if max_length != "NULL" && !max_length.is_empty() {
                format!("VARCHAR({})", max_length)
            } else {
                "VARCHAR".to_string()
            }
        }
        "character" => {
            if max_length != "NULL" && !max_length.is_empty() {
                format!("CHAR({})", max_length)
            } else {
                "CHAR".to_string()
            }
        }
        "integer" => "INTEGER".to_string(),
        "bigint" => "BIGINT".to_string(),
        "smallint" => "SMALLINT".to_string(),
        "boolean" => "BOOLEAN".to_string(),
        "text" => "TEXT".to_string(),
        "uuid" => "UUID".to_string(),
        "jsonb" => "JSONB".to_string(),
        "json" => "JSON".to_string(),
        "timestamp without time zone" => "TIMESTAMP".to_string(),
        "timestamp with time zone" => "TIMESTAMPTZ".to_string(),
        "date" => "DATE".to_string(),
        "numeric" => "NUMERIC".to_string(),
        "double precision" => "DOUBLE PRECISION".to_string(),
        "real" => "REAL".to_string(),
        "bytea" => "BYTEA".to_string(),
        "USER-DEFINED" => format!("\"{}\"", udt_name),
        _ => data_type.to_uppercase(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Init Command
// ═══════════════════════════════════════════════════════════════════════════════

/// Source for initializing a pgroller project.
#[derive(Debug, Clone)]
pub enum InitSource {
    /// Fresh project with empty baseline template
    Fresh,
    /// Snapshot schema from a live database
    Database(String),
    /// Import from a pg_dump file
    Dump {
        path: String,
        strip_dml: bool,
        keep_dml: bool,
    },
}

/// Initialize a new pgroller project.
pub async fn run_init(dir: &Path, source: InitSource, postgres_version: &str) -> Result<()> {
    ui::print_phase("Project Init");

    // Check if directory already has a pgroller project
    let baseline_dir = dir.join("0__baseline");
    if baseline_dir.exists() {
        return Err(PgrollerError::Config(format!(
            "Directory already contains a pgroller project: {}",
            baseline_dir.display()
        )));
    }

    match source {
        InitSource::Fresh => init_fresh(dir, postgres_version)?,
        InitSource::Database(ref uri) => init_from_database(dir, uri, postgres_version).await?,
        InitSource::Dump {
            ref path,
            strip_dml,
            keep_dml,
        } => init_from_dump(dir, path, strip_dml, keep_dml, postgres_version)?,
    }

    Ok(())
}

/// Create a fresh pgroller project with empty templates.
fn init_fresh(dir: &Path, postgres_version: &str) -> Result<()> {
    ui::print_subphase("creating project structure");

    let baseline_dir = dir.join("0__baseline");
    std::fs::create_dir_all(&baseline_dir)?;

    // up.sql template
    std::fs::write(
        baseline_dir.join("up.sql"),
        "-- Baseline schema: add your CREATE TABLE statements here.\n",
    )?;

    // test.sql template
    std::fs::write(
        baseline_dir.join("test.sql"),
        "-- Seed data and assertions for the baseline schema.\n-- INSERT INTO ... VALUES (...);\n-- @assert(query=\"SELECT count(*) FROM ...\", expected=\"...\")\n",
    )?;

    // pgroller.toml
    write_config(dir, postgres_version)?;

    ui::print_phase("Done");
    ui::print_info("created", &format!("{}/pgroller.toml", dir.display()));
    ui::print_info("created", &format!("{}/up.sql", baseline_dir.display()));
    ui::print_info("created", &format!("{}/test.sql", baseline_dir.display()));
    ui::print_subphase("edit 0__baseline/up.sql to define your schema");

    Ok(())
}

/// Initialize from a live database connection.
async fn init_from_database(dir: &Path, uri: &str, postgres_version: &str) -> Result<()> {
    ui::print_subphase("connecting to database");

    // Connect and dump schema using our existing schema snapshot + dump logic
    let conn_str = parse_postgres_uri(uri)?;
    let schema_sql = dump_schema(&conn_str, "public").await?;

    ui::print_subphase("writing baseline");

    let baseline_dir = dir.join("0__baseline");
    std::fs::create_dir_all(&baseline_dir)?;

    std::fs::write(baseline_dir.join("up.sql"), &schema_sql)?;

    std::fs::write(
        baseline_dir.join("test.sql"),
        "-- Add seed data and assertions for the baseline schema.\n",
    )?;

    write_config(dir, postgres_version)?;

    ui::print_phase("Done");
    ui::print_info("created", &format!("{}/pgroller.toml", dir.display()));
    ui::print_info(
        "created",
        &format!(
            "{}/up.sql ({} bytes)",
            baseline_dir.display(),
            schema_sql.len()
        ),
    );
    ui::print_info("created", &format!("{}/test.sql", baseline_dir.display()));
    ui::print_subphase("review 0__baseline/up.sql and add seed data to test.sql");

    Ok(())
}

/// Initialize from a pg_dump file.
fn init_from_dump(
    dir: &Path,
    dump_path: &str,
    strip_dml: bool,
    keep_dml: bool,
    postgres_version: &str,
) -> Result<()> {
    ui::print_subphase("parsing dump file");

    let content = std::fs::read_to_string(dump_path).map_err(|e| PgrollerError::Io(e))?;

    // Parse with pg_query to classify statements
    let parsed = pg_query::parse(&content)
        .map_err(|e| PgrollerError::Parse(format!("Failed to parse dump file: {}", e)))?;

    let mut ddl_statements: Vec<String> = Vec::new();
    let mut dml_count: usize = 0;

    for stmt in parsed.protobuf.stmts.iter() {
        let stmt_start = stmt.stmt_location as usize;
        let stmt_len = if stmt.stmt_len > 0 {
            stmt.stmt_len as usize
        } else {
            content.len() - stmt_start
        };
        let stmt_text = content[stmt_start..stmt_start + stmt_len]
            .trim()
            .to_string();

        let is_dml = if let Some(ref node) = stmt.stmt {
            if let Some(ref inner) = node.node {
                matches!(
                    inner,
                    pg_query::protobuf::node::Node::InsertStmt(_)
                        | pg_query::protobuf::node::Node::UpdateStmt(_)
                        | pg_query::protobuf::node::Node::DeleteStmt(_)
                        | pg_query::protobuf::node::Node::CopyStmt(_)
                )
            } else {
                false
            }
        } else {
            false
        };

        if is_dml {
            dml_count += 1;
        } else {
            ddl_statements.push(stmt_text);
        }
    }

    let ddl_count = ddl_statements.len();
    ui::print_info("DDL statements", &format!("{}", ddl_count));

    if dml_count > 0 {
        ui::print_info(
            "DML statements",
            &format!("{} (INSERT, UPDATE, DELETE, COPY)", dml_count),
        );

        let should_strip = if strip_dml {
            true
        } else if keep_dml {
            false
        } else {
            // Interactive: ask the user
            use std::io::{self, Write};
            println!();
            println!(
                "    {} DML statements are typically not needed in a baseline.",
                console::style("⚠").yellow()
            );
            println!("    The baseline should represent the schema structure, not data.");
            println!();
            print!("    Include DML in baseline? [y/N]: ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            let answer = input.trim().to_lowercase();
            answer != "y" && answer != "yes"
        };

        if should_strip {
            ui::print_subphase(&format!("stripped {} DML statements", dml_count));
        } else {
            // Re-parse and keep everything
            ui::print_subphase("keeping all statements (DDL + DML)");
            ddl_statements.clear();
            for stmt in parsed.protobuf.stmts.iter() {
                let stmt_start = stmt.stmt_location as usize;
                let stmt_len = if stmt.stmt_len > 0 {
                    stmt.stmt_len as usize
                } else {
                    content.len() - stmt_start
                };
                let stmt_text = content[stmt_start..stmt_start + stmt_len]
                    .trim()
                    .to_string();
                ddl_statements.push(stmt_text);
            }
        }
    }

    // Write the baseline
    ui::print_subphase("writing baseline");

    let baseline_dir = dir.join("0__baseline");
    std::fs::create_dir_all(&baseline_dir)?;

    let mut up_sql = String::from("-- Generated by pgroller init --from-dump\n\n");
    for stmt in &ddl_statements {
        up_sql.push_str(stmt);
        up_sql.push_str(";\n\n");
    }

    std::fs::write(baseline_dir.join("up.sql"), &up_sql)?;

    std::fs::write(
        baseline_dir.join("test.sql"),
        "-- Add seed data and assertions for the baseline schema.\n",
    )?;

    write_config(dir, postgres_version)?;

    ui::print_phase("Done");
    ui::print_info("created", &format!("{}/pgroller.toml", dir.display()));
    ui::print_info(
        "created",
        &format!("{}/up.sql ({} bytes)", baseline_dir.display(), up_sql.len()),
    );
    ui::print_info("created", &format!("{}/test.sql", baseline_dir.display()));
    ui::print_subphase("review 0__baseline/up.sql and add seed data to test.sql");

    Ok(())
}

/// Write pgroller.toml to the project directory.
fn write_config(dir: &Path, postgres_version: &str) -> Result<()> {
    let config_content = format!(
        r#"[migrations]
dir = "."

[database]
postgres_version = "{}"
extensions = []
schema = "public"

[test]
timeout = 30
continue_on_failure = true
# How to reset between tests: "drop_schema" (safe, default) or "savepoint" (fast)
reset_strategy = "drop_schema"
"#,
        postgres_version
    );

    std::fs::write(dir.join("pgroller.toml"), config_content)?;
    Ok(())
}

/// Parse a PostgreSQL URI into a connection string for tokio-postgres.
/// Supports: postgresql://user:pass@host:port/dbname
fn parse_postgres_uri(uri: &str) -> Result<String> {
    // tokio-postgres accepts both URI and key-value format
    // Just pass the URI directly — tokio-postgres supports it
    if uri.starts_with("postgresql://") || uri.starts_with("postgres://") {
        Ok(uri.to_string())
    } else {
        Err(PgrollerError::Config(format!(
            "Invalid PostgreSQL URI: must start with postgresql:// or postgres://\n  Got: {}",
            uri
        )))
    }
}
