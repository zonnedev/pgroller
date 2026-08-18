use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::{PgrollerError, Result};

#[derive(Debug, Clone)]
pub struct Migration {
    pub version: u64,
    pub description: String,
    pub path: PathBuf,
}

impl Migration {
    pub fn has_file(&self, name: &str) -> bool {
        self.path.join(name).exists()
    }
}

/// Discover migrations in the given directory.
///
/// Requirements:
/// - `0__baseline` must exist with `up.sql` (no `down.sql`)
/// - All other migrations must have `up.sql` and `down.sql`
/// - Optional files: `test-before-up.sql`, `test-after-up.sql`, `test-after-down.sql`
pub fn discover_migrations(dir: &Path) -> Result<Vec<Migration>> {
    if !dir.exists() {
        return Err(PgrollerError::Discovery(format!(
            "Migrations directory does not exist: {}",
            dir.display()
        )));
    }

    if !dir.is_dir() {
        return Err(PgrollerError::Discovery(format!(
            "Path is not a directory: {}",
            dir.display()
        )));
    }

    let pattern = Regex::new(r"^(\d+)__([a-z][a-z0-9_]*)$")?;
    let mut migrations: Vec<Migration> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut versions_seen: HashMap<u64, String> = HashMap::new();

    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let entry_path = entry.path();

        // Skip archive directory
        if dir_name == "archive" {
            continue;
        }

        let caps = match pattern.captures(&dir_name) {
            Some(c) => c,
            None => {
                errors.push(format!(
                    "Invalid migration folder name '{}': must match format <number>__<lowercase_description>",
                    dir_name
                ));
                continue;
            }
        };

        let version_str = &caps[1];
        let description = caps[2].to_string();

        if version_str.len() > 1 && version_str.starts_with('0') {
            errors.push(format!(
                "Invalid migration folder name '{}': version number must not have leading zeros",
                dir_name
            ));
            continue;
        }

        let version: u64 = version_str.parse().map_err(|_| {
            PgrollerError::Discovery(format!(
                "Invalid version number in folder '{}': not a valid u64",
                dir_name
            ))
        })?;

        if let Some(existing) = versions_seen.get(&version) {
            errors.push(format!(
                "Duplicate migration version {}: found in '{}' and '{}'",
                version, existing, dir_name
            ));
            continue;
        }
        versions_seen.insert(version, dir_name.clone());

        // Validate required files
        if version == 0 {
            // Baseline: up.sql required, down.sql forbidden
            if !entry_path.join("up.sql").exists() {
                errors.push(format!(
                    "Baseline '{}' is missing required file: up.sql",
                    dir_name
                ));
                continue;
            }
            if entry_path.join("down.sql").exists() {
                errors.push(format!(
                    "Baseline '{}' must NOT contain down.sql (there is no state before the baseline to roll back to)",
                    dir_name
                ));
                continue;
            }
        } else {
            // Regular migration: up.sql + down.sql required
            let mut missing: Vec<&str> = Vec::new();
            if !entry_path.join("up.sql").exists() {
                missing.push("up.sql");
            }
            if !entry_path.join("down.sql").exists() {
                missing.push("down.sql");
            }
            if !missing.is_empty() {
                errors.push(format!(
                    "Migration '{}' is missing required files: {}",
                    dir_name,
                    missing.join(", ")
                ));
                continue;
            }
        }

        migrations.push(Migration {
            version,
            description,
            path: entry_path,
        });
    }

    if !errors.is_empty() {
        return Err(PgrollerError::Discovery(format!(
            "Migration discovery errors:\n  - {}",
            errors.join("\n  - ")
        )));
    }

    migrations.sort_by_key(|m| m.version);

    if migrations.is_empty() || migrations[0].version != 0 {
        return Err(PgrollerError::Discovery(
            "Missing required 0__baseline migration. Every project must have a 0__baseline/ folder."
                .to_string(),
        ));
    }

    Ok(migrations)
}
