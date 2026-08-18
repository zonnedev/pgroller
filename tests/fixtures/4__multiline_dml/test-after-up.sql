SELECT pgroller_test.assert_equal('system user created', (SELECT count(*) FROM users WHERE status = 'system')::bigint, 1::bigint);
SELECT pgroller_test.assert_equal('admin user created', (SELECT count(*) FROM users WHERE status = 'admin')::bigint, 1::bigint);
SELECT pgroller_test.assert_equal('active migrated', (SELECT count(*) FROM users WHERE status = 'migrated')::bigint, 2::bigint);
SELECT pgroller_test.assert_equal('dave deleted', (SELECT count(*) FROM users WHERE name = 'Dave')::bigint, 0::bigint);
