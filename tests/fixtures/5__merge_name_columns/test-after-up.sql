SELECT pgroller_test.assert_true('alice migrated',
    EXISTS(SELECT 1 FROM users WHERE display_name = 'Alice Smith'));
SELECT pgroller_test.assert_true('bob migrated',
    EXISTS(SELECT 1 FROM users WHERE display_name = 'Bob Jones'));
SELECT pgroller_test.assert_true('name column gone',
    NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='users' AND column_name='name'));
