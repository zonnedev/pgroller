INSERT INTO tasks (project_id, title) VALUES (1, 'Ship migration test');
SELECT pgroller_test.assert_count('tasks table available', 'tasks', 1);
