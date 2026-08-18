-- Seed: users with various statuses to exercise all DML paths
INSERT INTO users (id, name, status) VALUES
    ('cccccccc-cccc-cccc-cccc-cccccccccccc', 'Charlie', 'active'),
    ('dddddddd-dddd-dddd-dddd-dddddddddddd', 'Dave', 'inactive'),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee', 'Eve', 'active');

-- After migration:
-- INSERT adds System+Admin (2 new rows)
-- UPDATE: active→migrated WHERE name != 'System' → Charlie,Eve become migrated (2)
-- DELETE: inactive AND not in admin subquery → Dave deleted
