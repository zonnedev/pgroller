SELECT pgroller_test.assert_false('legacy full name removed', EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'accounts' AND column_name = 'full_name'));
