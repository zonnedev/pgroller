SELECT pgroller_test.assert_equal('metadata defaults to empty object', (SELECT metadata FROM accounts WHERE email = 'owner10@example.test'), '{}'::jsonb);
