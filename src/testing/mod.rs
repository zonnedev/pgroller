//! Integration test framework for pgroller.
//!
//! Provides a builder-based API for defining migrations and running round-trip
//! tests programmatically, without needing fixture files on disk.

use crate::cli::{
    filter_sql_only, get_seeded_tables_with_conn, is_oid_constraint, snapshot_data_with_conn,
    snapshot_schema_with_conn, PGROLLER_TEST_SCHEMA,
};
use crate::container::PgContainer;
use crate::differ::{diff_data, diff_schemas};
use crate::executor::PersistentConn;
use crate::matcher::match_diffs;
use crate::parser::parse_annotations;

/// A single migration definition for testing.
pub struct Migration {
    pub name: String,
    pub up: String,
    pub down: String,
    pub before_up: Option<String>,
    pub after_up: Option<String>,
    pub after_down: Option<String>,
    pub no_schema_rollbacks: Vec<(String, String)>, // (target like "column=users.email", reason)
    pub no_data_rollbacks: Vec<(String, String)>,   // (table, reason)
}

impl Migration {
    /// Create a new migration with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            up: String::new(),
            down: String::new(),
            before_up: None,
            after_up: None,
            after_down: None,
            no_schema_rollbacks: Vec::new(),
            no_data_rollbacks: Vec::new(),
        }
    }

    /// Set the up (forward) SQL for this migration.
    pub fn up(mut self, sql: &str) -> Self {
        self.up = sql.to_string();
        self
    }

    /// Set the down (rollback) SQL for this migration.
    pub fn down(mut self, sql: &str) -> Self {
        self.down = sql.to_string();
        self
    }

    /// Set SQL to execute before the up migration (seed data).
    pub fn before_up(mut self, sql: &str) -> Self {
        self.before_up = Some(sql.to_string());
        self
    }

    /// Set SQL to execute after the up migration (assertions).
    pub fn after_up(mut self, sql: &str) -> Self {
        self.after_up = Some(sql.to_string());
        self
    }

    /// Set SQL to execute after the down migration (rollback assertions).
    pub fn after_down(mut self, sql: &str) -> Self {
        self.after_down = Some(sql.to_string());
        self
    }

    /// Add a @NoSchemaRollback annotation for this migration.
    /// Target format: "column=users.email", "table=events", "index=idx_name", etc.
    pub fn no_schema_rollback(mut self, target: &str, reason: &str) -> Self {
        self.no_schema_rollbacks
            .push((target.to_string(), reason.to_string()));
        self
    }

    /// Add a @NoDataRollback annotation for this migration.
    pub fn no_data_rollback(mut self, table: &str, reason: &str) -> Self {
        self.no_data_rollbacks
            .push((table.to_string(), reason.to_string()));
        self
    }
}

/// Builder for configuring and running a migration test.
pub struct MigrationTest {
    baseline: String,
    migrations: Vec<Migration>,
    postgres_version: String,
    extensions: Vec<String>,
}

impl MigrationTest {
    /// Create a new test with defaults: postgres 15, no extensions.
    pub fn new() -> Self {
        Self {
            baseline: String::new(),
            migrations: Vec::new(),
            postgres_version: "15".to_string(),
            extensions: Vec::new(),
        }
    }

    /// Set the baseline SQL (initial schema setup).
    pub fn baseline(mut self, sql: &str) -> Self {
        self.baseline = sql.to_string();
        self
    }

    /// Set the PostgreSQL version for the test container.
    pub fn postgres_version(mut self, v: &str) -> Self {
        self.postgres_version = v.to_string();
        self
    }

    /// Set extensions to install in the test container.
    pub fn extensions(mut self, exts: Vec<&str>) -> Self {
        self.extensions = exts.into_iter().map(|s| s.to_string()).collect();
        self
    }

    /// Add a migration to test.
    pub fn migration(mut self, m: Migration) -> Self {
        self.migrations.push(m);
        self
    }

    /// Run the test and return results.
    pub async fn run(self) -> TestResult {
        match self.run_impl().await {
            Ok(result) => result,
            Err(e) => {
                // If the whole run fails (container start, etc.), wrap in a single error
                TestResult {
                    migration_results: vec![MigrationResult {
                        name: "<setup>".to_string(),
                        status: MigrationStatus::Error(format!("Test setup failed: {}", e)),
                    }],
                }
            }
        }
    }

    async fn run_impl(self) -> crate::Result<TestResult> {
        // 1. Start PgContainer
        let container = PgContainer::start(&self.postgres_version).await?;
        let conn_str = container.connection_string();

        // 2. Install extensions if any
        if !self.extensions.is_empty() {
            container.install_extensions(&self.extensions).await?;
        }

        // 3. Create PersistentConn
        let conn = PersistentConn::connect(&conn_str).await?;

        // 4. Execute PGROLLER_TEST_SCHEMA
        conn.execute(PGROLLER_TEST_SCHEMA).await?;

        // 5. Apply baseline SQL
        if !self.baseline.is_empty() {
            conn.execute(&self.baseline).await?;
        }

        let mut migration_results = Vec::new();

        // 6. For each migration
        for (i, migration) in self.migrations.iter().enumerate() {
            let status = self.run_single_migration(&conn, &migration, i).await;
            migration_results.push(MigrationResult {
                name: migration.name.clone(),
                status,
            });
        }

        Ok(TestResult { migration_results })
    }

    async fn run_single_migration(
        &self,
        conn: &PersistentConn,
        migration: &Migration,
        index: usize,
    ) -> MigrationStatus {
        match self.run_single_migration_impl(conn, migration, index).await {
            Ok(status) => status,
            Err(e) => MigrationStatus::Error(e.to_string()),
        }
    }

    async fn run_single_migration_impl(
        &self,
        conn: &PersistentConn,
        migration: &Migration,
        index: usize,
    ) -> crate::Result<MigrationStatus> {
        let schema = "public";

        // a. DROP SCHEMA public CASCADE; CREATE SCHEMA public; SET search_path TO public;
        conn.execute(
            "DROP SCHEMA public CASCADE; CREATE SCHEMA public; SET search_path TO public;",
        )
        .await?;

        // b. Re-apply baseline
        if !self.baseline.is_empty() {
            conn.execute(&self.baseline).await?;
        }

        // c. Apply all prior migrations' up SQL
        for prior in &self.migrations[..index] {
            if !prior.up.is_empty() {
                conn.execute(&prior.up).await?;
            }
        }

        // d. Execute before_up SQL (if any)
        if let Some(ref before_up) = migration.before_up {
            conn.execute(before_up).await?;
        }

        // e. Snapshot schema A + data A
        let schema_a = snapshot_schema_with_conn(conn, schema).await?;
        let seeded_tables = get_seeded_tables_with_conn(conn, schema).await?;
        let data_a = snapshot_data_with_conn(conn, &seeded_tables, schema).await?;

        // f. Execute up SQL
        if !migration.up.is_empty() {
            if let Err(e) = conn.execute(&migration.up).await {
                return Ok(MigrationStatus::Error(format!("[up.sql] {}", e)));
            }
        }

        // g. Execute after_up SQL (if any) — if error, record as Error
        if let Some(ref after_up) = migration.after_up {
            if let Err(e) = conn.execute(after_up).await {
                return Ok(MigrationStatus::Error(format!("[test-after-up.sql] {}", e)));
            }
        }

        // h. Build down.sql content: annotations as comments + down SQL
        let mut down_content = String::new();
        for (target, reason) in &migration.no_schema_rollbacks {
            down_content.push_str(&format!(
                "-- @NoSchemaRollback({}, reason=\"{}\")\n",
                target, reason
            ));
        }
        for (table, reason) in &migration.no_data_rollbacks {
            down_content.push_str(&format!(
                "-- @NoDataRollback(table={}, reason=\"{}\")\n",
                table, reason
            ));
        }
        down_content.push_str(&migration.down);

        // i. Execute down SQL (filtered, no comments)
        let down_sql = filter_sql_only(&down_content);
        if !down_sql.trim().is_empty() {
            if let Err(e) = conn.execute(&down_sql).await {
                return Ok(MigrationStatus::Error(format!("[down.sql] {}", e)));
            }
        }

        // j. Execute after_down SQL (if any) — if error, record as Error
        if let Some(ref after_down) = migration.after_down {
            if let Err(e) = conn.execute(after_down).await {
                return Ok(MigrationStatus::Error(format!(
                    "[test-after-down.sql] {}",
                    e
                )));
            }
        }

        // k. Snapshot schema B + data B
        let schema_b = snapshot_schema_with_conn(conn, schema).await?;
        let data_b = snapshot_data_with_conn(conn, &seeded_tables, schema).await?;

        // l. Diff and filter OID constraints
        let mut schema_diff = diff_schemas(&schema_a, &schema_b);
        schema_diff
            .missing_constraints
            .retain(|c| !is_oid_constraint(&c.name));
        schema_diff
            .extra_constraints
            .retain(|c| !is_oid_constraint(&c.name));
        let data_diff = diff_data(&data_a, &data_b);

        // m. Parse annotations from the constructed down content
        let annotations = parse_annotations(&down_content)?;

        // n. match_diffs
        let match_result = match_diffs(&schema_diff, &data_diff, &annotations);

        // o. Record result
        if !match_result.uncovered.is_empty() {
            let uncovered_descriptions: Vec<String> = match_result
                .uncovered
                .iter()
                .map(|u| u.description.clone())
                .collect();
            return Ok(MigrationStatus::Fail {
                uncovered: uncovered_descriptions,
            });
        }

        if match_result.has_stale() {
            return Ok(MigrationStatus::Warning {
                stale_count: match_result.stale_count(),
            });
        }

        Ok(MigrationStatus::Pass {
            covered_count: match_result.covered.len(),
        })
    }
}

impl Default for MigrationTest {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of running the full test suite.
pub struct TestResult {
    pub migration_results: Vec<MigrationResult>,
}

/// Result of testing a single migration.
pub struct MigrationResult {
    pub name: String,
    pub status: MigrationStatus,
}

/// Status of a single migration test.
#[derive(Debug)]
pub enum MigrationStatus {
    /// Round-trip was clean, or all diffs covered by annotations.
    Pass { covered_count: usize },
    /// There are uncovered diffs (test failure).
    Fail { uncovered: Vec<String> },
    /// Stale annotations that don't match any diff.
    Warning { stale_count: usize },
    /// An error occurred during testing.
    Error(String),
}

impl TestResult {
    /// Panics if any migration didn't pass (including warnings, which are acceptable).
    pub fn assert_pass(&self) {
        for result in &self.migration_results {
            match &result.status {
                MigrationStatus::Pass { .. } | MigrationStatus::Warning { .. } => {}
                MigrationStatus::Fail { uncovered } => {
                    panic!(
                        "Migration '{}' failed with uncovered diffs:\n{}",
                        result.name,
                        uncovered.join("\n  ")
                    );
                }
                MigrationStatus::Error(msg) => {
                    panic!("Migration '{}' errored: {}", result.name, msg);
                }
            }
        }
    }

    /// Same as assert_pass.
    pub fn assert_all_pass(&self) {
        self.assert_pass();
    }

    /// Panics if all migrations passed (expects at least one failure).
    pub fn assert_fail(&self) {
        let has_failure = self
            .migration_results
            .iter()
            .any(|r| matches!(r.status, MigrationStatus::Fail { .. }));
        if !has_failure {
            panic!(
                "Expected at least one migration to fail, but all passed or had warnings/errors"
            );
        }
    }

    /// Assert a specific migration passed.
    pub fn assert_migration_passes(&self, name: &str) {
        let result = self.find_migration(name);
        match &result.status {
            MigrationStatus::Pass { .. } | MigrationStatus::Warning { .. } => {}
            MigrationStatus::Fail { uncovered } => {
                panic!(
                    "Expected migration '{}' to pass, but it failed with:\n  {}",
                    name,
                    uncovered.join("\n  ")
                );
            }
            MigrationStatus::Error(msg) => {
                panic!(
                    "Expected migration '{}' to pass, but it errored: {}",
                    name, msg
                );
            }
        }
    }

    /// Assert a specific migration failed.
    pub fn assert_migration_fails(&self, name: &str) {
        let result = self.find_migration(name);
        if !matches!(result.status, MigrationStatus::Fail { .. }) {
            panic!(
                "Expected migration '{}' to fail, but got: {:?}",
                name, result.status
            );
        }
    }

    /// Assert that a specific migration's uncovered diffs contain the given text.
    pub fn assert_uncovered_contains(&self, name: &str, text: &str) {
        let result = self.find_migration(name);
        match &result.status {
            MigrationStatus::Fail { uncovered } => {
                let found = uncovered.iter().any(|u| u.contains(text));
                if !found {
                    panic!(
                        "Expected uncovered diffs for '{}' to contain '{}', but got:\n  {}",
                        name,
                        text,
                        uncovered.join("\n  ")
                    );
                }
            }
            other => {
                panic!(
                    "Expected migration '{}' to fail (to check uncovered), but got: {:?}",
                    name, other
                );
            }
        }
    }

    fn find_migration(&self, name: &str) -> &MigrationResult {
        self.migration_results
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("No migration named '{}' in results", name))
    }
}
