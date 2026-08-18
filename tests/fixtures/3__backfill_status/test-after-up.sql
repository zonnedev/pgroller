SELECT pgroller_test.assert_equal('backfill worked', (SELECT status FROM users WHERE name = 'Eve'), 'active');
