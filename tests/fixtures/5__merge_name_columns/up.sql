ALTER TABLE users ADD COLUMN display_name VARCHAR(200);
UPDATE users SET display_name = name;
ALTER TABLE users DROP COLUMN name;
