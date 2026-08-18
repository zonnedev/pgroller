ALTER TABLE tasks ADD COLUMN due_date DATE;
UPDATE tasks SET due_date = CURRENT_DATE + 7;
