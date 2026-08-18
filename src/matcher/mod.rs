use crate::differ::{DataDiff, SchemaDiff};
use crate::parser::{Annotations, NoDataRollback, NoSchemaRollback, SchemaTarget};

/// Result of matching schema/data diffs against annotations.
#[derive(Debug, Clone, Default)]
pub struct MatchResult {
    pub covered: Vec<CoveredDiff>,
    pub uncovered: Vec<UncoveredDiff>,
    pub stale_schema: Vec<NoSchemaRollback>,
    pub stale_data: Vec<NoDataRollback>,
}

#[derive(Debug, Clone)]
pub struct CoveredDiff {
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct UncoveredDiff {
    pub description: String,
    pub suggestion: String,
}

impl MatchResult {
    pub fn has_stale(&self) -> bool {
        !self.stale_schema.is_empty() || !self.stale_data.is_empty()
    }

    pub fn stale_count(&self) -> usize {
        self.stale_schema.len() + self.stale_data.len()
    }
}

/// Match schema and data diffs against annotations.
pub fn match_diffs(
    schema_diff: &SchemaDiff,
    data_diff: &DataDiff,
    annotations: &Annotations,
) -> MatchResult {
    let mut result = MatchResult::default();
    let mut used_schema: Vec<bool> = vec![false; annotations.schema.len()];
    let mut used_data: Vec<bool> = vec![false; annotations.data.len()];

    // === SCHEMA DIFFS ===

    // Missing columns
    for col in &schema_diff.missing_columns {
        let target = format!("{}.{}", col.table, col.column);
        let desc = format!("Missing column: {}.{}", col.table, col.column);

        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Column, &target) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Table, &col.table)
        {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!(
                    "-- @NoSchemaRollback(column={}.{}, reason=\"TODO\")",
                    col.table, col.column
                ),
            });
        }
    }

    // Modified columns
    for col_diff in &schema_diff.modified_columns {
        let target = format!("{}.{}", col_diff.table, col_diff.column);
        let desc = format!("Modified column: {}.{}", col_diff.table, col_diff.column);

        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Column, &target) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else if let Some(idx) =
            find_schema(&annotations.schema, &SchemaTarget::Table, &col_diff.table)
        {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!(
                    "-- @NoSchemaRollback(column={}.{}, reason=\"TODO\")",
                    col_diff.table, col_diff.column
                ),
            });
        }
    }

    // Missing tables
    for table in &schema_diff.missing_tables {
        let desc = format!("Missing table: {}", table);
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Table, table) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!("-- @NoSchemaRollback(table={}, reason=\"TODO\")", table),
            });
        }
    }

    // Missing indexes
    for index in &schema_diff.missing_indexes {
        let desc = format!("Missing index: {}", index);
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Index, index) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else if find_table_covering_index(&annotations.schema, index).is_some() {
            let tidx = find_table_covering_index(&annotations.schema, index).unwrap();
            used_schema[tidx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!("-- @NoSchemaRollback(index={}, reason=\"TODO\")", index),
            });
        }
    }

    // Missing constraints
    for constraint in &schema_diff.missing_constraints {
        let target = format!("{}.{}", constraint.table, constraint.name);
        let desc = format!(
            "Missing constraint: {}.{}",
            constraint.table, constraint.name
        );

        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Constraint, &target) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else if let Some(idx) =
            find_schema(&annotations.schema, &SchemaTarget::Table, &constraint.table)
        {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!(
                    "-- @NoSchemaRollback(constraint={}.{}, reason=\"TODO\")",
                    constraint.table, constraint.name
                ),
            });
        }
    }

    // Missing types
    for type_name in &schema_diff.missing_types {
        let desc = format!("Missing type: {}", type_name);
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Type, type_name) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Table, type_name)
        {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!("-- @NoSchemaRollback(type={}, reason=\"TODO\")", type_name),
            });
        }
    }

    // Missing sequences
    for seq in &schema_diff.missing_sequences {
        let desc = format!("Missing sequence: {}", seq);
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Sequence, seq) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else if find_table_covering_index(&annotations.schema, seq).is_some() {
            let tidx = find_table_covering_index(&annotations.schema, seq).unwrap();
            used_schema[tidx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!("-- @NoSchemaRollback(sequence={}, reason=\"TODO\")", seq),
            });
        }
    }

    // Missing functions
    for func in &schema_diff.missing_functions {
        let desc = format!("Missing function: {}", func);
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Function, func) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!("-- @NoSchemaRollback(function={}, reason=\"TODO\")", func),
            });
        }
    }

    // Missing triggers
    for trigger in &schema_diff.missing_triggers {
        let target = format!("{}.{}", trigger.table, trigger.name);
        let desc = format!("Missing trigger: {}.{}", trigger.table, trigger.name);
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Trigger, &target) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
        } else {
            result.uncovered.push(UncoveredDiff {
                description: desc,
                suggestion: format!(
                    "-- @NoSchemaRollback(trigger={}.{}, reason=\"TODO\")",
                    trigger.table, trigger.name
                ),
            });
        }
    }

    // === DATA DIFFS ===

    for table_diff in &data_diff.table_diffs {
        let table = &table_diff.table;
        let has_changes = !table_diff.missing_rows.is_empty()
            || !table_diff.extra_rows.is_empty()
            || !table_diff.modified_rows.is_empty();

        if !has_changes {
            continue;
        }

        let desc = format!("Data difference in table: {}", table);

        // Check @NoSchemaRollback(table=X) — covers everything including data
        if let Some(idx) = find_schema(&annotations.schema, &SchemaTarget::Table, table) {
            used_schema[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
            continue;
        }

        // Check @NoDataRollback(table=X)
        if let Some(idx) = find_data(&annotations.data, table) {
            used_data[idx] = true;
            result.covered.push(CoveredDiff { description: desc });
            continue;
        }

        result.uncovered.push(UncoveredDiff {
            description: desc,
            suggestion: format!("-- @NoDataRollback(table={}, reason=\"TODO\")", table),
        });
    }

    // === STALE ANNOTATIONS ===

    for (i, ann) in annotations.schema.iter().enumerate() {
        if !used_schema[i] {
            result.stale_schema.push(ann.clone());
        }
    }

    for (i, ann) in annotations.data.iter().enumerate() {
        if !used_data[i] {
            result.stale_data.push(ann.clone());
        }
    }

    result
}

fn find_schema(
    annotations: &[NoSchemaRollback],
    target: &SchemaTarget,
    name: &str,
) -> Option<usize> {
    annotations
        .iter()
        .position(|a| &a.target == target && a.name == name)
}

fn find_data(annotations: &[NoDataRollback], table: &str) -> Option<usize> {
    annotations.iter().position(|a| a.table == table)
}

/// Check if an index/sequence name is covered by a table-level annotation.
fn find_table_covering_index(annotations: &[NoSchemaRollback], name: &str) -> Option<usize> {
    annotations.iter().position(|a| {
        a.target == SchemaTarget::Table && (name.starts_with(&a.name) || name.contains(&a.name))
    })
}
