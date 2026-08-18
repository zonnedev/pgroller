ALTER TABLE accounts ADD COLUMN display_name VARCHAR(160);
UPDATE accounts SET display_name = full_name;
