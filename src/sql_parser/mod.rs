use crate::{PgrollerError, Result};

/// A parsed SQL statement with its normalized form for comparison.
#[derive(Debug, Clone)]
pub struct ParsedStatement {
    /// The original SQL text
    pub original: String,
    /// The normalized/deparsed SQL (canonical form for comparison)
    pub normalized: String,
    /// Tables affected by this statement
    pub affected_tables: Vec<String>,
    /// Whether this is a DML statement (INSERT, UPDATE, DELETE)
    pub is_dml: bool,
}

/// Parse a SQL file into individual statements using pg_query.
/// Returns only DML statements (INSERT, UPDATE, DELETE) since DDL is handled by schema diff.
pub fn extract_dml_statements(sql: &str) -> Result<Vec<ParsedStatement>> {
    let parsed = pg_query::parse(sql)
        .map_err(|e| PgrollerError::Parse(format!("Failed to parse SQL: {}", e)))?;

    let mut statements = Vec::new();

    for stmt in parsed.protobuf.stmts.iter() {
        let node = match &stmt.stmt {
            Some(n) => n,
            None => continue,
        };

        let node_ref = match &node.node {
            Some(n) => n,
            None => continue,
        };

        let (is_dml, affected_tables) = classify_statement(node_ref);

        if !is_dml {
            continue;
        }

        // Get the original text for this statement
        let stmt_start = stmt.stmt_location as usize;
        let stmt_len = if stmt.stmt_len > 0 {
            stmt.stmt_len as usize
        } else {
            // Last statement — goes to end of input
            sql.len() - stmt_start
        };
        let original = sql[stmt_start..stmt_start + stmt_len].trim().to_string();

        // Normalize: deparse back to canonical SQL
        let normalized = normalize_sql(&original).unwrap_or_else(|_| original.clone());

        statements.push(ParsedStatement {
            original,
            normalized,
            affected_tables,
            is_dml: true,
        });
    }

    Ok(statements)
}

/// Normalize a SQL statement by parsing and deparsing it.
/// This produces a canonical form where whitespace, casing, etc. don't matter.
pub fn normalize_sql(sql: &str) -> Result<String> {
    // pg_query::deparse requires a full parse result
    let parsed = pg_query::parse(sql).map_err(|e| {
        PgrollerError::Parse(format!("Failed to parse SQL for normalization: {}", e))
    })?;

    let deparsed = pg_query::deparse(&parsed.protobuf)
        .map_err(|e| PgrollerError::Parse(format!("Failed to deparse SQL: {}", e)))?;

    Ok(deparsed)
}

/// Compare two SQL statements for structural equivalence.
/// Returns true if they represent the same operation regardless of formatting.
pub fn statements_match(stmt1: &str, stmt2: &str) -> bool {
    let norm1 = match normalize_sql(stmt1) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let norm2 = match normalize_sql(stmt2) {
        Ok(n) => n,
        Err(_) => return false,
    };

    norm1 == norm2
}

/// Classify a statement node and extract affected table names.
fn classify_statement(node: &pg_query::protobuf::node::Node) -> (bool, Vec<String>) {
    use pg_query::protobuf::node::Node;

    match node {
        Node::InsertStmt(stmt) => {
            let tables = extract_relation_name(stmt.relation.as_ref());
            (true, tables)
        }
        Node::UpdateStmt(stmt) => {
            let tables = extract_relation_name(stmt.relation.as_ref());
            (true, tables)
        }
        Node::DeleteStmt(stmt) => {
            let tables = extract_relation_name(stmt.relation.as_ref());
            (true, tables)
        }
        _ => (false, Vec::new()),
    }
}

/// Extract the table name from a RangeVar (relation reference).
fn extract_relation_name(relation: Option<&pg_query::protobuf::RangeVar>) -> Vec<String> {
    match relation {
        Some(rv) => {
            if rv.relname.is_empty() {
                Vec::new()
            } else {
                vec![rv.relname.clone()]
            }
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_simple_update() {
        let sql1 = "UPDATE users SET status = 'active' WHERE status IS NULL";
        let sql2 = "UPDATE  users  SET  status='active'  WHERE  status  IS  NULL";
        assert!(statements_match(sql1, sql2));
    }

    #[test]
    fn test_normalize_multiline() {
        let sql1 = "UPDATE users SET status = 'active' WHERE status IS NULL";
        let sql2 = "UPDATE users\n  SET status = 'active'\n  WHERE status IS NULL";
        assert!(statements_match(sql1, sql2));
    }

    #[test]
    fn test_different_statements_dont_match() {
        let sql1 = "UPDATE users SET status = 'active' WHERE status IS NULL";
        let sql2 = "UPDATE users SET status = 'inactive' WHERE status IS NULL";
        assert!(!statements_match(sql1, sql2));
    }

    #[test]
    fn test_extract_dml_from_mixed() {
        let sql = "CREATE TABLE foo (id INT);\nINSERT INTO foo VALUES (1);\nALTER TABLE foo ADD COLUMN bar TEXT;\nUPDATE foo SET bar = 'x';";
        let stmts = extract_dml_statements(sql).unwrap();
        assert_eq!(stmts.len(), 2); // INSERT and UPDATE only
        assert_eq!(stmts[0].affected_tables, vec!["foo"]);
        assert_eq!(stmts[1].affected_tables, vec!["foo"]);
    }

    #[test]
    fn test_extract_affected_table() {
        let sql = "UPDATE products SET status = 'active' WHERE id = 1;";
        let stmts = extract_dml_statements(sql).unwrap();
        assert_eq!(stmts.len(), 1);
        assert_eq!(stmts[0].affected_tables, vec!["products"]);
    }
}
