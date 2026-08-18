-- Restore the column structurally, but original values are lost
-- @NoSchemaRollback(column=users.name, reason="restored with DEFAULT empty string, original had no default")
-- @NoDataRollback(table=users, reason="original name values lost after merge into display_name")

ALTER TABLE users ADD COLUMN name VARCHAR(100) NOT NULL DEFAULT '';
ALTER TABLE users DROP COLUMN display_name;
