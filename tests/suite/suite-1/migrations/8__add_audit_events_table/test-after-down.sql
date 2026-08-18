SELECT pgroller_test.assert_false('audit events removed', EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'audit_events'));
