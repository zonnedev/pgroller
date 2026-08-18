use std::collections::HashMap;

use crate::executor::execute_query;
use crate::Result;

// ─── Schema Diff Types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub table: String,
    pub column: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDiff {
    pub table: String,
    pub column: String,
    pub before: ColumnInfo,
    pub after: ColumnInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintInfo {
    pub table: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerInfo {
    pub table: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct SchemaDiff {
    pub missing_tables: Vec<String>,
    pub extra_tables: Vec<String>,
    pub missing_columns: Vec<ColumnInfo>,
    pub extra_columns: Vec<ColumnInfo>,
    pub modified_columns: Vec<ColumnDiff>,
    pub missing_indexes: Vec<String>,
    pub extra_indexes: Vec<String>,
    pub missing_constraints: Vec<ConstraintInfo>,
    pub extra_constraints: Vec<ConstraintInfo>,
    pub missing_functions: Vec<String>,
    pub extra_functions: Vec<String>,
    pub missing_sequences: Vec<String>,
    pub extra_sequences: Vec<String>,
    pub missing_triggers: Vec<TriggerInfo>,
    pub extra_triggers: Vec<TriggerInfo>,
    pub missing_types: Vec<String>,
    pub extra_types: Vec<String>,
}

impl SchemaDiff {
    pub fn is_empty(&self) -> bool {
        self.missing_tables.is_empty()
            && self.extra_tables.is_empty()
            && self.missing_columns.is_empty()
            && self.extra_columns.is_empty()
            && self.modified_columns.is_empty()
            && self.missing_indexes.is_empty()
            && self.extra_indexes.is_empty()
            && self.missing_constraints.is_empty()
            && self.extra_constraints.is_empty()
            && self.missing_functions.is_empty()
            && self.extra_functions.is_empty()
            && self.missing_sequences.is_empty()
            && self.extra_sequences.is_empty()
            && self.missing_triggers.is_empty()
            && self.extra_triggers.is_empty()
            && self.missing_types.is_empty()
            && self.extra_types.is_empty()
    }
}

// ─── Schema Snapshot ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct SchemaSnapshot {
    pub tables: Vec<String>,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<String>,
    pub constraints: Vec<ConstraintInfo>,
    pub functions: Vec<String>,
    pub sequences: Vec<String>,
    pub triggers: Vec<TriggerInfo>,
    pub types: Vec<String>,
}

/// Capture the current schema state from information_schema.
pub async fn snapshot_schema(conn_str: &str, schema: &str) -> Result<SchemaSnapshot> {
    let mut snapshot = SchemaSnapshot::default();

    // Tables
    let query = format!(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = '{}' AND table_type = 'BASE TABLE' \
         ORDER BY table_name",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.tables = rows.into_iter().map(|r| r[0].clone()).collect();

    // Columns
    let query = format!(
        "SELECT table_name, column_name, data_type, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = '{}' \
         ORDER BY table_name, ordinal_position",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
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

    // Indexes
    let query = format!(
        "SELECT indexname FROM pg_indexes \
         WHERE schemaname = '{}' \
         ORDER BY indexname",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.indexes = rows.into_iter().map(|r| r[0].clone()).collect();

    // Constraints
    let query = format!(
        "SELECT tc.table_name, tc.constraint_name \
         FROM information_schema.table_constraints tc \
         WHERE tc.table_schema = '{}' \
         ORDER BY tc.table_name, tc.constraint_name",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.constraints = rows
        .into_iter()
        .map(|r| ConstraintInfo {
            table: r[0].clone(),
            name: r[1].clone(),
        })
        .collect();

    // Functions
    let query = format!(
        "SELECT routine_name FROM information_schema.routines \
         WHERE routine_schema = '{}' \
         ORDER BY routine_name",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.functions = rows.into_iter().map(|r| r[0].clone()).collect();

    // Sequences
    let query = format!(
        "SELECT sequence_name FROM information_schema.sequences \
         WHERE sequence_schema = '{}' \
         ORDER BY sequence_name",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.sequences = rows.into_iter().map(|r| r[0].clone()).collect();

    // Triggers
    let query = format!(
        "SELECT event_object_table, trigger_name \
         FROM information_schema.triggers \
         WHERE trigger_schema = '{}' \
         ORDER BY event_object_table, trigger_name",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.triggers = rows
        .into_iter()
        .map(|r| TriggerInfo {
            table: r[0].clone(),
            name: r[1].clone(),
        })
        .collect();

    // User-defined types (enums, composites)
    let query = format!(
        "SELECT t.typname FROM pg_type t \
         JOIN pg_namespace n ON t.typnamespace = n.oid \
         WHERE n.nspname = '{}' \
         AND t.typtype IN ('e', 'c') \
         ORDER BY t.typname",
        schema
    );
    let rows = execute_query(conn_str, &query).await?;
    snapshot.types = rows.into_iter().map(|r| r[0].clone()).collect();

    Ok(snapshot)
}

/// Compare two schema snapshots and produce a diff.
pub fn diff_schemas(before: &SchemaSnapshot, after: &SchemaSnapshot) -> SchemaDiff {
    let mut diff = SchemaDiff::default();

    // Tables
    diff.missing_tables = before
        .tables
        .iter()
        .filter(|t| !after.tables.contains(t))
        .cloned()
        .collect();
    diff.extra_tables = after
        .tables
        .iter()
        .filter(|t| !before.tables.contains(t))
        .cloned()
        .collect();

    // Columns
    let before_cols: HashMap<(&str, &str), &ColumnInfo> = before
        .columns
        .iter()
        .map(|c| ((c.table.as_str(), c.column.as_str()), c))
        .collect();
    let after_cols: HashMap<(&str, &str), &ColumnInfo> = after
        .columns
        .iter()
        .map(|c| ((c.table.as_str(), c.column.as_str()), c))
        .collect();

    for (key, col) in &before_cols {
        if !after_cols.contains_key(key) {
            diff.missing_columns.push((*col).clone());
        } else {
            let after_col = after_cols[key];
            if col.data_type != after_col.data_type
                || col.is_nullable != after_col.is_nullable
                || col.default != after_col.default
            {
                diff.modified_columns.push(ColumnDiff {
                    table: col.table.clone(),
                    column: col.column.clone(),
                    before: (*col).clone(),
                    after: after_col.clone(),
                });
            }
        }
    }
    for (key, col) in &after_cols {
        if !before_cols.contains_key(key) {
            diff.extra_columns.push((*col).clone());
        }
    }

    // Indexes
    diff.missing_indexes = before
        .indexes
        .iter()
        .filter(|i| !after.indexes.contains(i))
        .cloned()
        .collect();
    diff.extra_indexes = after
        .indexes
        .iter()
        .filter(|i| !before.indexes.contains(i))
        .cloned()
        .collect();

    // Constraints
    let before_constraints: Vec<(&str, &str)> = before
        .constraints
        .iter()
        .map(|c| (c.table.as_str(), c.name.as_str()))
        .collect();
    let after_constraints: Vec<(&str, &str)> = after
        .constraints
        .iter()
        .map(|c| (c.table.as_str(), c.name.as_str()))
        .collect();

    diff.missing_constraints = before
        .constraints
        .iter()
        .filter(|c| !after_constraints.contains(&(c.table.as_str(), c.name.as_str())))
        .cloned()
        .collect();
    diff.extra_constraints = after
        .constraints
        .iter()
        .filter(|c| !before_constraints.contains(&(c.table.as_str(), c.name.as_str())))
        .cloned()
        .collect();

    // Functions
    diff.missing_functions = before
        .functions
        .iter()
        .filter(|f| !after.functions.contains(f))
        .cloned()
        .collect();
    diff.extra_functions = after
        .functions
        .iter()
        .filter(|f| !before.functions.contains(f))
        .cloned()
        .collect();

    // Sequences
    diff.missing_sequences = before
        .sequences
        .iter()
        .filter(|s| !after.sequences.contains(s))
        .cloned()
        .collect();
    diff.extra_sequences = after
        .sequences
        .iter()
        .filter(|s| !before.sequences.contains(s))
        .cloned()
        .collect();

    // Triggers
    let before_triggers: Vec<(&str, &str)> = before
        .triggers
        .iter()
        .map(|t| (t.table.as_str(), t.name.as_str()))
        .collect();
    let after_triggers: Vec<(&str, &str)> = after
        .triggers
        .iter()
        .map(|t| (t.table.as_str(), t.name.as_str()))
        .collect();

    diff.missing_triggers = before
        .triggers
        .iter()
        .filter(|t| !after_triggers.contains(&(t.table.as_str(), t.name.as_str())))
        .cloned()
        .collect();
    diff.extra_triggers = after
        .triggers
        .iter()
        .filter(|t| !before_triggers.contains(&(t.table.as_str(), t.name.as_str())))
        .cloned()
        .collect();

    // Types
    diff.missing_types = before
        .types
        .iter()
        .filter(|t| !after.types.contains(t))
        .cloned()
        .collect();
    diff.extra_types = after
        .types
        .iter()
        .filter(|t| !before.types.contains(t))
        .cloned()
        .collect();

    diff
}

// ─── Data Diff Types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellDiff {
    pub column: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowDiff {
    pub key: String,
    pub differences: Vec<CellDiff>,
}

#[derive(Debug, Clone, Default)]
pub struct TableDataDiff {
    pub table: String,
    pub missing_rows: Vec<Vec<String>>,
    pub extra_rows: Vec<Vec<String>>,
    pub modified_rows: Vec<RowDiff>,
}

#[derive(Debug, Clone, Default)]
pub struct DataDiff {
    pub table_diffs: Vec<TableDataDiff>,
}

impl DataDiff {
    pub fn is_empty(&self) -> bool {
        self.table_diffs.is_empty()
    }

    pub fn affected_tables(&self) -> Vec<&str> {
        self.table_diffs.iter().map(|d| d.table.as_str()).collect()
    }
}

// ─── Data Snapshot ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TableSnapshot {
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct DataSnapshot {
    pub tables: Vec<TableSnapshot>,
}

/// Capture data from the specified tables.
///
/// Queries each table with `SELECT * ORDER BY <first_column>` and stores rows
/// as vectors of string representations.
pub async fn snapshot_data(
    conn_str: &str,
    tables: &[String],
    schema: &str,
) -> Result<DataSnapshot> {
    let mut snapshot = DataSnapshot::default();

    for table in tables {
        // Get column names
        let col_query = format!(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = '{}' AND table_name = '{}' \
             ORDER BY ordinal_position",
            schema, table
        );
        let col_rows = execute_query(conn_str, &col_query).await?;
        let columns: Vec<String> = col_rows.into_iter().map(|r| r[0].clone()).collect();

        if columns.is_empty() {
            continue;
        }

        // Get data sorted by first column
        let data_query = format!(
            "SELECT * FROM \"{}\".\"{}\" ORDER BY \"{}\"",
            schema, table, columns[0]
        );
        let rows = execute_query(conn_str, &data_query)
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

/// Compare two data snapshots and produce a diff.
pub fn diff_data(before: &DataSnapshot, after: &DataSnapshot) -> DataDiff {
    let mut diff = DataDiff::default();

    let before_map: HashMap<&str, &TableSnapshot> = before
        .tables
        .iter()
        .map(|t| (t.table.as_str(), t))
        .collect();
    let after_map: HashMap<&str, &TableSnapshot> =
        after.tables.iter().map(|t| (t.table.as_str(), t)).collect();

    // Check all tables that appear in either snapshot
    let mut all_tables: Vec<&str> = before_map.keys().chain(after_map.keys()).copied().collect();
    all_tables.sort();
    all_tables.dedup();

    for table in all_tables {
        let before_table = before_map.get(table);
        let after_table = after_map.get(table);

        let table_diff = match (before_table, after_table) {
            (Some(b), Some(a)) => diff_table_data(table, b, a),
            (Some(b), None) => TableDataDiff {
                table: table.to_string(),
                missing_rows: b.rows.clone(),
                extra_rows: Vec::new(),
                modified_rows: Vec::new(),
            },
            (None, Some(a)) => TableDataDiff {
                table: table.to_string(),
                missing_rows: Vec::new(),
                extra_rows: a.rows.clone(),
                modified_rows: Vec::new(),
            },
            (None, None) => continue,
        };

        if !table_diff.missing_rows.is_empty()
            || !table_diff.extra_rows.is_empty()
            || !table_diff.modified_rows.is_empty()
        {
            diff.table_diffs.push(table_diff);
        }
    }

    diff
}

fn diff_table_data(table: &str, before: &TableSnapshot, after: &TableSnapshot) -> TableDataDiff {
    let mut table_diff = TableDataDiff {
        table: table.to_string(),
        missing_rows: Vec::new(),
        extra_rows: Vec::new(),
        modified_rows: Vec::new(),
    };

    // Use first column as the key for row matching
    let before_keyed: HashMap<&str, &Vec<String>> = before
        .rows
        .iter()
        .filter_map(|r| r.first().map(|k| (k.as_str(), r)))
        .collect();
    let after_keyed: HashMap<&str, &Vec<String>> = after
        .rows
        .iter()
        .filter_map(|r| r.first().map(|k| (k.as_str(), r)))
        .collect();

    // Find missing and modified rows
    for (key, before_row) in &before_keyed {
        match after_keyed.get(key) {
            None => {
                table_diff.missing_rows.push((*before_row).clone());
            }
            Some(after_row) => {
                if before_row != after_row {
                    let mut differences = Vec::new();
                    let columns = &before.columns;
                    for (i, col) in columns.iter().enumerate() {
                        let b_val = before_row.get(i).map(|s| s.as_str()).unwrap_or("NULL");
                        let a_val = after_row.get(i).map(|s| s.as_str()).unwrap_or("NULL");
                        if b_val != a_val {
                            differences.push(CellDiff {
                                column: col.clone(),
                                before: b_val.to_string(),
                                after: a_val.to_string(),
                            });
                        }
                    }
                    if !differences.is_empty() {
                        table_diff.modified_rows.push(RowDiff {
                            key: key.to_string(),
                            differences,
                        });
                    }
                }
            }
        }
    }

    // Find extra rows
    for (key, after_row) in &after_keyed {
        if !before_keyed.contains_key(key) {
            table_diff.extra_rows.push((*after_row).clone());
        }
    }

    table_diff
}
