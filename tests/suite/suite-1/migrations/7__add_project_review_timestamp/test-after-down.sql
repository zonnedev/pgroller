SELECT pgroller_test.assert_false('review timestamp removed', EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'projects' AND column_name = 'last_reviewed_at'));
