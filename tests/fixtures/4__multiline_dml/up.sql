INSERT INTO users (id, name, status)
VALUES
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa', 'System', 'system'),
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb', 'Admin', 'admin');

UPDATE users
SET status = 'migrated'
WHERE status = 'active'
  AND name != 'System';

DELETE FROM users
WHERE status = 'inactive'
  AND name NOT IN (
    SELECT name FROM users WHERE status = 'admin'
  );
