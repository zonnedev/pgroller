SELECT pgroller_test.assert_false('due date removed', EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'tasks' AND column_name = 'due_date'));
