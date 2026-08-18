SELECT pgroller_test.assert_true('email column gone', NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='email'));
