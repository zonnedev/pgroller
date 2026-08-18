SELECT pgroller_test.assert_equal('default status', (SELECT status FROM users WHERE name = 'Charlie'), 'active');
