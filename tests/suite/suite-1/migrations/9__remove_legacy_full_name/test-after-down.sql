SELECT pgroller_test.assert_equal('lossy rollback uses documented default', (SELECT full_name FROM accounts WHERE email = 'legacy@example.test'), '');
