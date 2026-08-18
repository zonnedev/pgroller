SELECT pgroller_test.assert_equal('display name backfilled', (SELECT display_name FROM accounts WHERE email = 'profile@example.test'), 'Profile User');
