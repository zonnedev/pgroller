ALTER TABLE projects ADD COLUMN project_code VARCHAR(24);
UPDATE projects SET project_code = 'PLATFORM-001';
