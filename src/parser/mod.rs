use regex::Regex;

use crate::{PgrollerError, Result};

/// Target type for schema rollback annotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaTarget {
    Table,
    Column,
    Index,
    Constraint,
    Type,
    Sequence,
    Function,
    Trigger,
}

impl SchemaTarget {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "table" => Some(Self::Table),
            "column" => Some(Self::Column),
            "index" => Some(Self::Index),
            "constraint" => Some(Self::Constraint),
            "type" => Some(Self::Type),
            "sequence" => Some(Self::Sequence),
            "function" => Some(Self::Function),
            "trigger" => Some(Self::Trigger),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Column => "column",
            Self::Index => "index",
            Self::Constraint => "constraint",
            Self::Type => "type",
            Self::Sequence => "sequence",
            Self::Function => "function",
            Self::Trigger => "trigger",
        }
    }
}

/// A parsed @NoSchemaRollback annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoSchemaRollback {
    pub target: SchemaTarget,
    pub name: String,
    pub reason: String,
}

/// A parsed @NoDataRollback annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoDataRollback {
    pub table: String,
    pub reason: String,
}

/// All annotations from a down.sql file.
#[derive(Debug, Clone, Default)]
pub struct Annotations {
    pub schema: Vec<NoSchemaRollback>,
    pub data: Vec<NoDataRollback>,
}

/// Parse annotations from down.sql content.
///
/// Supports:
/// - `-- @NoSchemaRollback(<target>=<name>, reason="...")`
/// - `-- @NoDataRollback(table=<name>, reason="...")`
pub fn parse_annotations(content: &str) -> Result<Annotations> {
    let mut annotations = Annotations::default();

    let schema_re = Regex::new(
        r#"--\s*@NoSchemaRollback\(\s*(\w+)\s*=\s*([^,]+?)\s*,\s*reason\s*=\s*"([^"]+)"\s*\)"#,
    )?;

    let data_re = Regex::new(
        r#"--\s*@NoDataRollback\(\s*table\s*=\s*([^,]+?)\s*,\s*reason\s*=\s*"([^"]+)"\s*\)"#,
    )?;

    for line in content.lines() {
        let trimmed = line.trim();

        if !trimmed.starts_with("--") {
            continue;
        }

        if trimmed.contains("@NoSchemaRollback") {
            if let Some(caps) = schema_re.captures(trimmed) {
                let target_str = &caps[1];
                let name = caps[2].trim().to_string();
                let reason = caps[3].to_string();

                let target = SchemaTarget::from_str(target_str).ok_or_else(|| {
                    PgrollerError::Parse(format!(
                        "Unknown target '{}' in @NoSchemaRollback",
                        target_str
                    ))
                })?;

                annotations.schema.push(NoSchemaRollback {
                    target,
                    name,
                    reason,
                });
            } else {
                return Err(PgrollerError::Parse(format!(
                    "Malformed @NoSchemaRollback: {}",
                    trimmed
                )));
            }
            continue;
        }

        if trimmed.contains("@NoDataRollback") {
            if let Some(caps) = data_re.captures(trimmed) {
                let table = caps[1].trim().to_string();
                let reason = caps[2].to_string();
                annotations.data.push(NoDataRollback { table, reason });
            } else {
                return Err(PgrollerError::Parse(format!(
                    "Malformed @NoDataRollback: {}",
                    trimmed
                )));
            }
            continue;
        }
    }

    Ok(annotations)
}
