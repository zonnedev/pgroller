ALTER TABLE accounts ADD COLUMN metadata JSONB NOT NULL DEFAULT '{}'::jsonb;
