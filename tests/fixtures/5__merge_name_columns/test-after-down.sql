SELECT pgroller_test.assert_true('name column restored',
    EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='name'));
SELECT pgroller_test.assert_true('display_name column gone',
    NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='display_name'));
