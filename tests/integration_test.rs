use std::path::PathBuf;

use pgroller::discovery::discover_migrations;
use pgroller::parser::{parse_annotations, SchemaTarget};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// =============================================================================
// Discovery tests
// =============================================================================

#[test]
fn test_discover_finds_all_migrations() {
    let migrations = discover_migrations(&fixtures_dir()).unwrap();
    assert_eq!(migrations.len(), 6);
}

#[test]
fn test_discover_migrations_sorted_by_version() {
    let migrations = discover_migrations(&fixtures_dir()).unwrap();
    let versions: Vec<u64> = migrations.iter().map(|m| m.version).collect();
    assert_eq!(versions, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn test_discover_baseline_is_first() {
    let migrations = discover_migrations(&fixtures_dir()).unwrap();
    assert_eq!(migrations[0].version, 0);
    assert_eq!(migrations[0].description, "baseline");
}

#[test]
fn test_discover_baseline_has_no_down_sql() {
    let migrations = discover_migrations(&fixtures_dir()).unwrap();
    assert!(!migrations[0].has_file("down.sql"));
}

#[test]
fn test_discover_regular_migrations_have_up_and_down() {
    let migrations = discover_migrations(&fixtures_dir()).unwrap();
    for m in migrations.iter().skip(1) {
        assert!(m.has_file("up.sql"), "Missing up.sql in {}", m.description);
        assert!(
            m.has_file("down.sql"),
            "Missing down.sql in {}",
            m.description
        );
    }
}

#[test]
fn test_discover_optional_test_files_detected() {
    let migrations = discover_migrations(&fixtures_dir()).unwrap();
    // baseline has test-before-up.sql
    assert!(migrations[0].has_file("test-before-up.sql"));
    // migration 1 has both test files
    assert!(migrations[1].has_file("test-before-up.sql"));
    assert!(migrations[1].has_file("test-after-up.sql"));
}

#[test]
fn test_discover_nonexistent_directory_errors() {
    let result = discover_migrations(&PathBuf::from("/nonexistent/path"));
    assert!(result.is_err());
}

#[test]
fn test_discover_file_instead_of_directory_errors() {
    let file_path = fixtures_dir().join("pgroller.toml");
    let result = discover_migrations(&file_path);
    assert!(result.is_err());
}

// =============================================================================
// Annotation parser tests (@NoSchemaRollback / @NoDataRollback)
// =============================================================================

#[test]
fn test_parse_no_schema_rollback_column() {
    let content = r#"-- @NoSchemaRollback(column=users.email, reason="data moved to JSONB")"#;
    let annotations = parse_annotations(content).unwrap();
    assert_eq!(annotations.schema.len(), 1);
    assert_eq!(annotations.schema[0].target, SchemaTarget::Column);
    assert_eq!(annotations.schema[0].name, "users.email");
    assert_eq!(annotations.schema[0].reason, "data moved to JSONB");
}

#[test]
fn test_parse_no_schema_rollback_table() {
    let content = r#"-- @NoSchemaRollback(table=old_events, reason="archived to S3")"#;
    let annotations = parse_annotations(content).unwrap();
    assert_eq!(annotations.schema.len(), 1);
    assert_eq!(annotations.schema[0].target, SchemaTarget::Table);
    assert_eq!(annotations.schema[0].name, "old_events");
}

#[test]
fn test_parse_no_data_rollback() {
    let content = r#"-- @NoDataRollback(table=users, reason="backfilled values not reversible")"#;
    let annotations = parse_annotations(content).unwrap();
    assert_eq!(annotations.data.len(), 1);
    assert_eq!(annotations.data[0].table, "users");
    assert_eq!(
        annotations.data[0].reason,
        "backfilled values not reversible"
    );
}

#[test]
fn test_parse_mixed_annotations() {
    let content = r#"-- @NoSchemaRollback(column=users.email, reason="dropped")
-- @NoSchemaRollback(index=idx_email, reason="depends on column")
-- @NoDataRollback(table=users, reason="data lost")
ALTER TABLE users ADD COLUMN email VARCHAR(255);
"#;
    let annotations = parse_annotations(content).unwrap();
    assert_eq!(annotations.schema.len(), 2);
    assert_eq!(annotations.data.len(), 1);
}

#[test]
fn test_parse_no_annotations_returns_empty() {
    let content = "ALTER TABLE users DROP COLUMN status;\n";
    let annotations = parse_annotations(content).unwrap();
    assert!(annotations.schema.is_empty());
    assert!(annotations.data.is_empty());
}

#[test]
fn test_parse_plain_comments_ignored() {
    let content = "-- This is a regular comment\n-- Another comment\nDROP TABLE foo;\n";
    let annotations = parse_annotations(content).unwrap();
    assert!(annotations.schema.is_empty());
    assert!(annotations.data.is_empty());
}

#[test]
fn test_parse_malformed_no_schema_rollback_errors() {
    let content = "-- @NoSchemaRollback(bad format)\n";
    let result = parse_annotations(content);
    assert!(result.is_err());
}

#[test]
fn test_parse_malformed_no_data_rollback_errors() {
    let content = "-- @NoDataRollback(invalid)\n";
    let result = parse_annotations(content);
    assert!(result.is_err());
}

#[test]
fn test_parse_unknown_schema_target_errors() {
    let content = r#"-- @NoSchemaRollback(foobar=something, reason="test")"#;
    let result = parse_annotations(content);
    assert!(result.is_err());
}
