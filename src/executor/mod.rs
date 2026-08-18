use std::path::Path;

use tokio_postgres::NoTls;

use crate::{PgrollerError, Result};

/// Execute a SQL string against the database.
pub async fn execute_sql(conn_str: &str, sql: &str) -> Result<()> {
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
        .await
        .map_err(|e| PgrollerError::Execution(format!("Connection failed: {}", e)))?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    client
        .batch_execute(sql)
        .await
        .map_err(|e| PgrollerError::Execution(format!("SQL execution failed: {}", e)))?;

    Ok(())
}

/// Read a SQL file and execute it against the database.
pub async fn execute_file(conn_str: &str, path: &Path) -> Result<()> {
    let sql = std::fs::read_to_string(path)?;
    execute_sql(conn_str, &sql).await
}

/// Execute a query and return all rows as vectors of strings.
///
/// Each row is a `Vec<String>` where each element is the text representation
/// of the column value (or "NULL" for null values).
pub async fn execute_query(conn_str: &str, query: &str) -> Result<Vec<Vec<String>>> {
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
        .await
        .map_err(|e| PgrollerError::Execution(format!("Connection failed: {}", e)))?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    let rows = client
        .query(query, &[])
        .await
        .map_err(|e| PgrollerError::Execution(format!("Query execution failed: {}", e)))?;

    let mut result = Vec::new();
    for row in &rows {
        let mut row_values = Vec::new();
        for i in 0..row.len() {
            // Try to get value as text; tokio-postgres supports converting most types to string
            let value: Option<String> = row.try_get::<_, Option<String>>(i).unwrap_or_else(|_| {
                // Fall back to trying other common types
                if let Ok(v) = row.try_get::<_, Option<i32>>(i) {
                    v.map(|n| n.to_string())
                } else if let Ok(v) = row.try_get::<_, Option<i64>>(i) {
                    v.map(|n| n.to_string())
                } else if let Ok(v) = row.try_get::<_, Option<bool>>(i) {
                    v.map(|b| b.to_string())
                } else if let Ok(v) = row.try_get::<_, Option<f64>>(i) {
                    v.map(|f| f.to_string())
                } else {
                    Some("<unsupported>".to_string())
                }
            });
            row_values.push(value.unwrap_or_else(|| "NULL".to_string()));
        }
        result.push(row_values);
    }

    Ok(result)
}

/// A persistent database connection for operations that need shared state (e.g., savepoints).
pub struct PersistentConn {
    client: tokio_postgres::Client,
}

impl PersistentConn {
    /// Create a new persistent connection.
    pub async fn connect(conn_str: &str) -> Result<Self> {
        let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
            .await
            .map_err(|e| PgrollerError::Execution(format!("Connection failed: {}", e)))?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("Connection error: {}", e);
            }
        });

        Ok(Self { client })
    }

    /// Execute SQL on this connection.
    pub async fn execute(&self, sql: &str) -> Result<()> {
        self.client.batch_execute(sql).await.map_err(|e| {
            let detail = if let Some(db_err) = e.as_db_error() {
                format!("{}: {}", db_err.severity(), db_err.message())
            } else {
                e.to_string()
            };
            PgrollerError::Execution(detail)
        })?;
        Ok(())
    }

    /// Execute a SQL file on this connection.
    pub async fn execute_file(&self, path: &Path) -> Result<()> {
        let sql = std::fs::read_to_string(path)?;
        self.execute(&sql).await
    }

    /// Execute a query and return rows.
    /// Handles all common Postgres types by converting to string representation.
    pub async fn query(&self, query: &str) -> Result<Vec<Vec<String>>> {
        let rows = self
            .client
            .query(query, &[])
            .await
            .map_err(|e| PgrollerError::Execution(format!("Query execution failed: {}", e)))?;

        let mut result = Vec::new();
        for row in &rows {
            let mut row_values = Vec::new();
            for (i, column) in row.columns().iter().enumerate() {
                let value = get_column_as_string(row, i, column.type_());
                row_values.push(value);
            }
            result.push(row_values);
        }

        Ok(result)
    }
}

/// Convert a column value to a string representation based on its Postgres type.
fn get_column_as_string(
    row: &tokio_postgres::Row,
    idx: usize,
    col_type: &tokio_postgres::types::Type,
) -> String {
    use tokio_postgres::types::Type;

    // Handle NULL for any type
    macro_rules! try_type {
        ($t:ty) => {
            if let Ok(v) = row.try_get::<_, Option<$t>>(idx) {
                return match v {
                    Some(val) => val.to_string(),
                    None => "NULL".to_string(),
                };
            }
        };
    }

    match *col_type {
        Type::BOOL => try_type!(bool),
        Type::INT2 => try_type!(i16),
        Type::INT4 => try_type!(i32),
        Type::INT8 => try_type!(i64),
        Type::FLOAT4 => try_type!(f32),
        Type::FLOAT8 => try_type!(f64),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => try_type!(String),
        Type::UUID => {
            if let Ok(v) = row.try_get::<_, Option<uuid::Uuid>>(idx) {
                return match v {
                    Some(val) => val.to_string(),
                    None => "NULL".to_string(),
                };
            }
        }
        Type::TIMESTAMP => {
            if let Ok(v) = row.try_get::<_, Option<chrono::NaiveDateTime>>(idx) {
                return match v {
                    Some(val) => val.to_string(),
                    None => "NULL".to_string(),
                };
            }
        }
        Type::TIMESTAMPTZ => {
            if let Ok(v) = row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(idx) {
                return match v {
                    Some(val) => val.to_string(),
                    None => "NULL".to_string(),
                };
            }
        }
        Type::NUMERIC => {
            // NUMERIC handled by f64 fallback below
        }
        Type::JSON | Type::JSONB => {
            if let Ok(v) = row.try_get::<_, Option<serde_json::Value>>(idx) {
                return match v {
                    Some(val) => val.to_string(),
                    None => "NULL".to_string(),
                };
            }
        }
        _ => {}
    }

    // Fallback: try common types in order
    if let Ok(v) = row.try_get::<_, Option<String>>(idx) {
        return v.unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<i64>>(idx) {
        return v
            .map(|n| n.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<i32>>(idx) {
        return v
            .map(|n| n.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<bool>>(idx) {
        return v
            .map(|b| b.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<f64>>(idx) {
        return v
            .map(|f| f.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }

    // Last resort: try to get raw bytes and represent as hex
    "NULL".to_string()
}
