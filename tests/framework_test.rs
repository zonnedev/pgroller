//! Integration tests for the pgroller testing framework.
//!
//! These tests require Docker to be running (Testcontainers).

use pgroller::testing::{Migration, MigrationTest};

#[tokio::test]
async fn test_add_column_round_trip_clean() {
    // Replicates 1__add_status_column
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, email VARCHAR(255) NOT NULL UNIQUE);")
        .migration(
            Migration::new("add_status_column")
                .up("ALTER TABLE users ADD COLUMN status VARCHAR(50) DEFAULT 'active';")
                .down("ALTER TABLE users DROP COLUMN status;")
                .before_up("INSERT INTO users (name, email) VALUES ('Alice', 'alice@test.com');")
                .after_up("SELECT pgroller_test.assert_equal('default status', (SELECT status FROM users WHERE name = 'Alice'), 'active');")
        )
        .run().await;

    result.assert_pass();
}

#[tokio::test]
async fn test_drop_column_needs_annotation() {
    // Replicates 2__drop_email_column without annotations — should fail
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, email VARCHAR(255) NOT NULL UNIQUE);")
        .migration(
            Migration::new("drop_email")
                .up("ALTER TABLE users DROP COLUMN email;")
                .down("")
                .before_up("INSERT INTO users (name, email) VALUES ('Alice', 'alice@test.com');")
        )
        .run().await;

    result.assert_fail();
    result.assert_uncovered_contains("drop_email", "users.email");
}

#[tokio::test]
async fn test_drop_column_with_annotation_passes() {
    // Same but with proper annotations covering all diffs
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, email VARCHAR(255) NOT NULL UNIQUE);")
        .migration(
            Migration::new("drop_email")
                .up("ALTER TABLE users DROP COLUMN email;")
                .down("")
                .no_schema_rollback("column=users.email", "data cannot be reconstructed")
                .no_schema_rollback("index=users_email_key", "depends on dropped column")
                .no_schema_rollback("constraint=users.users_email_key", "UNIQUE on dropped column")
                .no_data_rollback("users", "seed data includes email which no longer exists")
                .before_up("INSERT INTO users (name, email) VALUES ('Alice', 'alice@test.com');")
        )
        .run().await;

    result.assert_pass();
}

#[tokio::test]
async fn test_data_backfill_needs_annotation() {
    // Replicates 3__backfill_status without annotation — should fail
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, status VARCHAR(50));")
        .migration(
            Migration::new("backfill_status")
                .up("UPDATE users SET status = 'active' WHERE status IS NULL;")
                .down("")
                .before_up("INSERT INTO users (name, status) VALUES ('Alice', NULL);")
        )
        .run().await;

    result.assert_fail();
    result.assert_uncovered_contains("backfill_status", "users");
}

#[tokio::test]
async fn test_data_backfill_with_annotation_passes() {
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, status VARCHAR(50));")
        .migration(
            Migration::new("backfill_status")
                .up("UPDATE users SET status = 'active' WHERE status IS NULL;")
                .down("")
                .no_data_rollback("users", "cannot distinguish original NULLs")
                .before_up("INSERT INTO users (name, status) VALUES ('Alice', NULL);")
                .after_up("SELECT pgroller_test.assert_equal('backfilled', (SELECT status FROM users WHERE name = 'Alice'), 'active');")
        )
        .run().await;

    result.assert_pass();
}

#[tokio::test]
async fn test_multiline_dml_with_annotations() {
    // Replicates 4__multiline_dml
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, status VARCHAR(50) DEFAULT 'active');")
        .migration(
            Migration::new("multiline_dml")
                .up(r#"
                    INSERT INTO users (id, name, status) VALUES
                        ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa', 'System', 'system'),
                        ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb', 'Admin', 'admin');
                    UPDATE users SET status = 'migrated' WHERE status = 'active' AND name != 'System';
                    DELETE FROM users WHERE status = 'inactive' AND name NOT IN (SELECT name FROM users WHERE status = 'admin');
                "#)
                .down("")
                .no_data_rollback("users", "DML operations cannot be fully reversed")
                .before_up(r#"
                    INSERT INTO users (name, status) VALUES ('Charlie', 'active'), ('Dave', 'inactive'), ('Eve', 'active');
                "#)
                .after_up(r#"
                    SELECT pgroller_test.assert_equal('system created', (SELECT count(*) FROM users WHERE status = 'system')::bigint, 1::bigint);
                    SELECT pgroller_test.assert_equal('admin created', (SELECT count(*) FROM users WHERE status = 'admin')::bigint, 1::bigint);
                "#)
        )
        .run().await;

    result.assert_pass();
}

#[tokio::test]
async fn test_partial_rollback_data_loss() {
    // Replicates 5__merge_name_columns
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name VARCHAR(100) NOT NULL, status VARCHAR(50));")
        .migration(
            Migration::new("merge_name")
                .up(r#"
                    ALTER TABLE users ADD COLUMN display_name VARCHAR(200);
                    UPDATE users SET display_name = name;
                    ALTER TABLE users DROP COLUMN name;
                "#)
                .down(r#"
                    ALTER TABLE users ADD COLUMN name VARCHAR(100) NOT NULL DEFAULT '';
                    ALTER TABLE users DROP COLUMN display_name;
                "#)
                .no_schema_rollback("column=users.name", "restored with DEFAULT, original had no default")
                .no_data_rollback("users", "original name values lost after merge")
                .before_up("INSERT INTO users (name, status) VALUES ('Alice Smith', 'active'), ('Bob Jones', 'active');")
                .after_up(r#"
                    SELECT pgroller_test.assert_true('alice migrated', EXISTS(SELECT 1 FROM users WHERE display_name = 'Alice Smith'));
                    SELECT pgroller_test.assert_true('name column gone', NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='name'));
                "#)
                .after_down(r#"
                    SELECT pgroller_test.assert_true('name column restored', EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='name'));
                    SELECT pgroller_test.assert_true('display_name gone', NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='display_name'));
                "#)
        )
        .run().await;

    result.assert_pass();
}

#[tokio::test]
async fn test_drop_table_annotation_covers_all() {
    // Table-level annotation covers all sub-components
    let result = MigrationTest::new()
        .baseline(r#"
            CREATE TABLE users (id SERIAL PRIMARY KEY, name VARCHAR(100) NOT NULL);
            CREATE TABLE events (id SERIAL PRIMARY KEY, user_id INT REFERENCES users(id), name VARCHAR(100), created_at TIMESTAMP DEFAULT NOW());
            CREATE INDEX idx_events_user ON events(user_id);
        "#)
        .migration(
            Migration::new("drop_events")
                .up("DROP TABLE events;")
                .down("")
                .no_schema_rollback("table=events", "archived to cold storage")
                .before_up("INSERT INTO users (name) VALUES ('Alice'); INSERT INTO events (user_id, name) VALUES (1, 'login');")
        )
        .run().await;

    result.assert_pass();
}

#[tokio::test]
async fn test_stale_annotation_produces_warning() {
    // Annotation doesn't match any diff — stale warning, but Warning counts as passing
    let result = MigrationTest::new()
        .baseline("CREATE TABLE users (id SERIAL PRIMARY KEY, name VARCHAR(100));")
        .migration(
            Migration::new("add_column")
                .up("ALTER TABLE users ADD COLUMN age INT;")
                .down("ALTER TABLE users DROP COLUMN age;")
                .no_data_rollback("users", "this is stale - no data diff exists"),
        )
        .run()
        .await;

    // Warning is treated as acceptable (passes) in pgroller
    result.assert_pass();
}
