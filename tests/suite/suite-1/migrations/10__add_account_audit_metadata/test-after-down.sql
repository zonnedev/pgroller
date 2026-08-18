SELECT pgroller_test.assert_false('metadata removed', EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'accounts' AND column_name = 'metadata'));
