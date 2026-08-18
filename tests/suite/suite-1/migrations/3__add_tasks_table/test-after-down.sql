SELECT pgroller_test.assert_false('tasks table removed', EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'tasks'));
