-- Seed: user with NULL status to test backfill
-- Schema at this point: id (UUID), name (VARCHAR), status (VARCHAR) — email already dropped
INSERT INTO users (id, name, status) VALUES
    ('55555555-5555-5555-5555-555555555555', 'Eve', NULL);

