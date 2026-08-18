SELECT pgroller_test.assert_false('project code removed', EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'projects' AND column_name = 'project_code'));
