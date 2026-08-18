SELECT pgroller_test.assert_false('labels removed', EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'labels'));
SELECT pgroller_test.assert_false('task labels removed', EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'task_labels'));
