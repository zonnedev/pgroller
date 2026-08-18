-- The original full names cannot be reconstructed after this migration.
-- @NoSchemaRollback(column=accounts.full_name, reason="legacy full_name values were replaced by display_name")
-- @NoDataRollback(table=accounts, reason="the rollback restores an empty full_name value because the original values were removed")
ALTER TABLE accounts ADD COLUMN full_name VARCHAR(120) NOT NULL DEFAULT '';
