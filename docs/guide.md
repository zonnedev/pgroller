# pgroller

**SQL migration rollback validator for PostgreSQL.**

pgroller proves your database migrations are reversible before you deploy them. It executes each migration forward and backward against a real PostgreSQL instance, compares schema and data snapshots, and reports what can't be rolled back.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Concepts](#concepts)
- [Project Structure](#project-structure)
- [Configuration](#configuration)
- [Commands](#commands)
  - [pgroller new](#pgroller-new)
  - [pgroller init](#pgroller-init)
  - [pgroller test](#pgroller-test)
  - [pgroller migrate](#pgroller-migrate)
  - [pgroller rollback](#pgroller-rollback)
  - [pgroller status](#pgroller-status)
  - [pgroller verify](#pgroller-verify)
  - [pgroller baseline](#pgroller-baseline)
- [Annotations](#annotations)
  - [@NoSchemaRollback](#noschemarollback)
  - [@NoDataRollback](#nodatarollback)
- [Test Files](#test-files)
  - [test-before-up.sql](#test-before-upsql)
  - [test-after-up.sql](#test-after-upsql)
  - [test-after-down.sql](#test-after-downsql)
- [Assertion Functions](#assertion-functions)
- [Reset Strategies](#reset-strategies)
- [Workflow](#workflow)
- [FAQ](#faq)

---

## Quick Start

```bash
# Initialize a new project
pgroller init ./db/migrations

# Create your baseline
# Edit db/migrations/0__baseline/up.sql with your initial schema

# Create a migration
mkdir db/migrations/1__add_users_table
# Write up.sql, down.sql, optionally test files

# Test rollback safety
pgroller test

# Apply to a database
pgroller migrate --database "postgresql://user:pass@localhost:5432/mydb" --accept

# Check status
pgroller status --database "postgresql://user:pass@localhost:5432/mydb"
```

---

## Concepts

### Round-Trip Testing

pgroller's core idea: for every migration, execute `up.sql → down.sql` and verify the database returns to its previous state. If it doesn't, either fix the rollback or explicitly document what's irreversible.

### Annotations

When a migration is intentionally irreversible (dropping a column, deleting data), you annotate it in `down.sql`. This is a conscious decision, not an oversight.

### Schema vs Data Diffs

After a round-trip (up→down), pgroller detects two types of differences:

- **Schema diffs**: columns, tables, indexes, constraints, types, sequences, functions, triggers that don't match the pre-migration state
- **Data diffs**: rows that were inserted, updated, or deleted by the migration and not restored by the rollback

---

## Project Structure

```
db/migrations/
├── pgroller.toml                   # Configuration
├── 0__baseline/                    # Required: initial schema
│   ├── up.sql                      # CREATE TABLE statements
│   ├── test-before-up.sql          # Optional: seed data
│   ├── test-after-up.sql           # Optional: assertions
│   └── test-after-down.sql         # Optional: rollback assertions
├── 1__add_status_column/           # First migration
│   ├── up.sql                      # Production migration
│   ├── down.sql                    # Production rollback + annotations
│   ├── test-before-up.sql          # Optional: seed data
│   ├── test-after-up.sql           # Optional: assertions after migration
│   └── test-after-down.sql         # Optional: assertions after rollback
├── 2__drop_email_column/
│   ├── up.sql
│   ├── down.sql
│   └── test-before-up.sql
└── ...
```

### Folder Naming

```
{version}__{description}
```

- `version`: positive integer, no leading zeros (except `0` for baseline)
- `__`: double underscore separator
- `description`: lowercase, alphanumeric, underscores only

**Valid:** `0__baseline`, `1__add_users`, `14__drop_legacy_events`  
**Invalid:** `V1__create`, `001__foo`, `1__Create-Users`

### File Requirements

| Folder | Required | Forbidden |
|--------|----------|-----------|
| `0__baseline` | `up.sql` | `down.sql` |
| `N__*` (N ≥ 1) | `up.sql`, `down.sql` | — |

Optional files (any migration): `test-before-up.sql`, `test-after-up.sql`, `test-after-down.sql`

---

## Configuration

`pgroller.toml` at the migrations root:

```toml
[migrations]
dir = "."                    # Path to migration folders (relative to this file)

[database]
postgres_version = "15"      # PostgreSQL version for test containers
extensions = []              # Extensions to install (e.g., ["uuid-ossp", "pgcrypto"])
schema = "public"            # Database schema to use

[test]
timeout = 30                 # Timeout per migration test (seconds)
continue_on_failure = true   # Continue testing after a test failure
reset_strategy = "drop_schema"  # "drop_schema" (safe) or "savepoint" (fast)
```

### Reset Strategies

| Strategy | How it works | Performance | Best for |
|----------|-------------|-------------|----------|
| `drop_schema` | Drops and recreates schema, re-applies all prior migrations per test | O(N²) | CI, correctness |
| `savepoint` | Keeps schema between tests, uses PostgreSQL savepoints to isolate | O(N) | Local development, speed |

Both produce identical results. `savepoint` is faster for large migration sets.

---

## Commands

### pgroller new

Create a new migration folder with auto-incremented version number.

```bash
pgroller new "add audit log"
pgroller new "drop legacy events" --accept
```

**Behavior:**

1. Normalizes the name (lowercase, spaces → underscores)
2. Finds the next version number (highest existing + 1)
3. Shows what it will create, asks for confirmation
4. Creates the folder with all files

**Output:**

```
  ▸ New Migration

    Will create:
      5__add_audit_log/
      ├── up.sql
      ├── down.sql
      ├── test-before-up.sql
      ├── test-after-up.sql
      └── test-after-down.sql

    Proceed? [Y/n]: y

    ✓ Created 5__add_audit_log/
```

**Options:**

| Flag | Description |
|------|-------------|
| `--accept` | Skip confirmation prompt |
| `-c, --config <file>` | Path to config file |

---

### pgroller init

Initialize a new pgroller project.

```bash
# Fresh project (empty baseline template)
pgroller init ./db/migrations

# From a live database
pgroller init ./db/migrations --from-database "postgresql://user:pass@host:5432/db"

# From a pg_dump file
pgroller init ./db/migrations --from-dump ./schema.sql
pgroller init ./db/migrations --from-dump ./full_dump.sql --strip-dml
pgroller init ./db/migrations --from-dump ./full_dump.sql --keep-dml
```

**Options:**

| Flag | Description |
|------|-------------|
| `--from-database <URI>` | Snapshot schema from a live database |
| `--from-dump <file>` | Import from a pg_dump SQL file |
| `--strip-dml` | Always remove DML from dump (non-interactive) |
| `--keep-dml` | Always keep DML in dump (non-interactive) |
| `--postgres-version <ver>` | PostgreSQL version for config (default: 15) |

---

### pgroller test

Test migration rollbacks for round-trip safety. Spins up a Testcontainers PostgreSQL instance and validates each migration.

```bash
pgroller test
pgroller test --config path/to/pgroller.toml
```

**Test cycle per migration:**

```
1. Reset database to state N-1 (apply all prior up.sql)
2. Execute test-before-up.sql (seed data, if exists)
3. Snapshot schema + data (state A)
4. Execute up.sql (migration)
5. Execute test-after-up.sql (assertions, if exists)
6. Execute down.sql (rollback)
7. Execute test-after-down.sql (rollback assertions, if exists)
8. Snapshot schema + data (state B)
9. Diff A vs B → uncovered diffs are failures
```

**Output:**

```
  ● 31 migrations: 31 passed  (8.05s)
```

Failures show exactly what's uncovered and suggest the annotation to add:

```
  ✗ 2__drop_email — uncovered diffs:

    ╭─ Missing column: users.email
    ╰→ -- @NoSchemaRollback(column=users.email, reason="TODO")
```

---

### pgroller migrate

Apply pending migrations to a database.

```bash
# Interactive (shows plan, asks confirmation)
pgroller migrate --database "postgresql://user:pass@host:5432/db"

# Non-interactive (CI/scripts)
pgroller migrate --database "..." --accept

# Preview only
pgroller migrate --database "..." --dry-run

# With schema verification after
pgroller migrate --database "..." --accept --verify
```

**Behavior:**

1. Shows all pending migrations with rollback annotations as warnings
2. Asks for confirmation (unless `--accept`)
3. Applies each migration in its own transaction
4. Records version + checksum in `pgroller_history` table
5. Stops on first failure

> **Note:** Only `up.sql` is executed against your database. Test files (`test-before-up.sql`, `test-after-up.sql`, `test-after-down.sql`) are never executed during `migrate` — they only run locally during `pgroller test`.

**Safety:**
- Advisory lock prevents concurrent migrations
- Checksum verification detects if someone modified an already-applied migration
- Annotations shown before execution so operator knows what's irreversible

---

### pgroller rollback

Rollback applied migrations.

```bash
# Rollback last migration (interactive)
pgroller rollback --database "postgresql://user:pass@host:5432/db"

# Rollback last 3 migrations
pgroller rollback --database "..." --steps 3

# Non-interactive
pgroller rollback --database "..." --steps 2 --accept

# Preview
pgroller rollback --database "..." --steps 2 --dry-run
```

**Behavior:**

1. Shows migrations to rollback with annotations as warnings
2. Asks for confirmation (unless `--accept`)
3. Applies each `down.sql` in reverse order within a transaction
4. Removes entries from `pgroller_history`
5. Automatically verifies schema after rollback (via `--verify`, default: true)

---

### pgroller status

Show migration status against a database.

```bash
pgroller status --database "postgresql://user:pass@host:5432/db"
```

**Output:**

```
  ✓ applied 0__baseline
  ✓ applied 1__add_status_column
  ✓ applied 2__drop_email_column
  ○ pending 3__backfill_status
  ○ pending 4__multiline_dml

    │ applied: 3
    │ pending: 2
```

---

### pgroller verify

Compare a live database schema against the expected state (migrations applied to a fresh reference container).

```bash
pgroller verify --database "postgresql://user:pass@host:5432/db"
```

**What it does:**

1. Connects to target database, reads `pgroller_history` to determine current version
2. Spins up a fresh Testcontainers PostgreSQL
3. Applies all migrations 0..N to the reference
4. Compares schemas between target and reference
5. Reports any drift

**Output (clean):**

```
  ✓ Schema matches expected state (version 31)
```

**Output (drift detected):**

```
  ✗ Schema drift detected:

    ╭─ Extra column: users.temp_flag (in target, not in reference)
    ╭─ Missing index: idx_users_status (in reference, not in target)
```

---

### pgroller baseline

Collapse all existing migrations into a new baseline.

```bash
# Preview
pgroller baseline --dry-run

# Execute
pgroller baseline
```

**What it does:**

1. Spins up Testcontainers PostgreSQL
2. Applies all migrations in order
3. Dumps the final schema as new `0__baseline/up.sql`
4. Moves old migrations to `archive/`

Useful when you have 100+ migrations and want a clean starting point.

---

## Annotations

Annotations live in `down.sql` only. They tell pgroller: "this difference after rollback is expected and intentional."

When `pgroller test` runs, it executes `up.sql` then `down.sql` and compares the database state before and after. If anything is different, pgroller reports it as a failure — unless an annotation covers that specific difference.

Without annotations, you're forced to either write a perfect rollback or face test failures. With annotations, you explicitly document what's irreversible and why.

---

### @NoSchemaRollback

Declares that a structural change to the database (a column, table, index, etc.) will not be restored by the rollback.

**Syntax:**

```sql
-- @NoSchemaRollback(<target>=<name>, reason="<explanation>")
```

**Target types:** `table`, `column`, `index`, `constraint`, `type`, `sequence`, `function`, `trigger`

---

#### Example: Dropping a column

**up.sql:**
```sql
ALTER TABLE users DROP COLUMN email;
```

**down.sql (without annotation):**
```sql
-- Nothing here — we can't restore the data
```

**pgroller test output (FAILS):**
```
  ✗ 2__drop_email — uncovered diffs:

    ╭─ Missing column: users.email
    ╰→ -- @NoSchemaRollback(column=users.email, reason="TODO")

    ╭─ Missing index: users_email_key
    ╰→ -- @NoSchemaRollback(index=users_email_key, reason="TODO")

    ╭─ Missing constraint: users.users_email_key
    ╰→ -- @NoSchemaRollback(constraint=users.users_email_key, reason="TODO")
```

pgroller is telling you: "after running up.sql then down.sql, the `email` column, its index, and its UNIQUE constraint are gone. You haven't accounted for this." It even suggests the exact annotation to paste.

**down.sql (with annotations):**
```sql
-- @NoSchemaRollback(column=users.email, reason="email data cannot be reconstructed after drop")
-- @NoSchemaRollback(index=users_email_key, reason="index depends on dropped column")
-- @NoSchemaRollback(constraint=users.users_email_key, reason="UNIQUE constraint on dropped column")
```

**pgroller test output (PASSES):**
```
  ✓ 2__drop_email — 3 annotated diffs
```

The "3 annotated diffs" means: pgroller detected 3 differences after rollback, and all 3 are covered by annotations. This is a pass — you've consciously documented the irreversibility.

---

#### Example: Dropping a table

**up.sql:**
```sql
DROP TABLE legacy_events;
```

**down.sql (with annotation):**
```sql
-- @NoSchemaRollback(table=legacy_events, reason="archived to S3, table no longer needed")
```

**Why `table=` is special:** A table-level annotation covers EVERYTHING belonging to that table — all its columns, indexes, constraints, sequences, and even data. You don't need to annotate each column individually.

**pgroller test output (PASSES):**
```
  ✓ 5__drop_legacy — 14 annotated diffs
```

Those 14 diffs are all the columns, indexes, constraints, etc. that belonged to `legacy_events`. One annotation covers them all.

---

#### Example: Partial rollback (column added, data dropped)

**up.sql:**
```sql
ALTER TABLE users ADD COLUMN display_name VARCHAR(100);
UPDATE users SET display_name = first_name || ' ' || last_name;
ALTER TABLE users DROP COLUMN first_name;
ALTER TABLE users DROP COLUMN last_name;
```

**down.sql:**
```sql
-- Restore the columns structurally, but the original data is lost
-- @NoDataRollback(table=users, reason="original first_name/last_name values lost after merge into display_name")

ALTER TABLE users ADD COLUMN first_name VARCHAR(100);
ALTER TABLE users ADD COLUMN last_name VARCHAR(100);
ALTER TABLE users DROP COLUMN display_name;
```

The rollback restores the schema (columns exist again), but the original values of `first_name` and `last_name` are gone — they were merged into `display_name` which is now dropped. The `@NoDataRollback` documents this data loss.

---

### @NoDataRollback

Declares that row data in a table will be different after rollback — rows were inserted, updated, or deleted by the migration and the rollback doesn't reverse the data change.

**Syntax:**

```sql
-- @NoDataRollback(table=<name>, reason="<explanation>")
```

---

#### Example: Data backfill

**up.sql:**
```sql
UPDATE users SET status = 'active' WHERE status IS NULL;
```

**down.sql (without annotation):**
```sql
-- Can't reverse: we don't know which rows were originally NULL
```

**test-before-up.sql (seed data to exercise this):**
```sql
INSERT INTO users (id, name, status) VALUES (1, 'Alice', NULL);
INSERT INTO users (id, name, status) VALUES (2, 'Bob', 'active');
```

**pgroller test output (FAILS):**
```
  ✗ 3__backfill_status — uncovered diffs:

    ╭─ Data difference in table: users
    ╰→ -- @NoDataRollback(table=users, reason="TODO")
```

pgroller detected that after up→down, the `users` table has different data than before (Alice's status is `'active'` instead of `NULL`). The rollback didn't restore it.

**down.sql (with annotation):**
```sql
-- @NoDataRollback(table=users, reason="cannot distinguish originally-NULL from backfilled values")
```

**pgroller test output (PASSES):**
```
  ✓ 3__backfill_status — 1 annotated diffs
```

---

#### Example: Destructive DELETE

**up.sql:**
```sql
DELETE FROM events WHERE created_at < '2023-01-01';
```

**down.sql (with annotation):**
```sql
-- @NoDataRollback(table=events, reason="deleted historical events cannot be recovered")
```

**test-before-up.sql (seed data that will be deleted):**
```sql
INSERT INTO events (id, name, created_at) VALUES (1, 'old_event', '2022-06-15');
INSERT INTO events (id, name, created_at) VALUES (2, 'new_event', '2024-03-20');
```

After up→down: `old_event` is gone (deleted by migration, not restored by rollback). The annotation covers this expected data loss.

---

#### Example: INSERT that can't be undone

**up.sql:**
```sql
INSERT INTO users (id, name, status)
VALUES ('system-001', 'SystemBot', 'system');
```

**down.sql:**
```sql
-- The system user must remain — removing it would break the application
-- @NoDataRollback(table=users, reason="system user cannot be removed once created")
```

After up→down: the system user row still exists (wasn't deleted by rollback). The annotation acknowledges this is intentional.

---

### Stale Annotations

If you add an annotation but there's no matching diff after rollback, pgroller warns you:

```
  ⚠ 5__something — stale annotations (no matching diff):
    • @NoDataRollback(table=users)
```

This means either:
- The rollback actually works (annotation is unnecessary — remove it)
- Your seed data doesn't exercise the DML path (the UPDATE/DELETE matched zero rows — fix your `test-before-up.sql`)

Stale annotations are warnings, not failures. They don't block your tests but signal that something might be wrong with your test coverage.

---

### Summary

| Annotation | Covers | Use when |
|-----------|--------|----------|
| `@NoSchemaRollback(table=X)` | All schema + data for table X | Dropping a table |
| `@NoSchemaRollback(column=X.Y)` | Column difference | Dropping/altering a column |
| `@NoSchemaRollback(index=X)` | Index difference | Dropping an index |
| `@NoSchemaRollback(constraint=X.Y)` | Constraint difference | Dropping a constraint |
| `@NoDataRollback(table=X)` | Data differences in table X | INSERT/UPDATE/DELETE not reversed |

---

## Test Files

All test files are **optional**. They're pure SQL — no special syntax.

> **Important:** Test files (`test-before-up.sql`, `test-after-up.sql`, `test-after-down.sql`) are **only executed locally** during `pgroller test`. They run against a disposable Testcontainers instance — never against your real database. The `pgroller migrate` and `pgroller rollback` commands only execute `up.sql` and `down.sql` against your target database. Your test scripts, seed data, and assertions never touch production.

### test-before-up.sql

Runs BEFORE the migration. Seeds the database with data that the migration will act on.

```sql
-- Insert data that exercises the migration's behavior
INSERT INTO users (id, name, status) VALUES
    ('aaa', 'Alice', NULL),
    ('bbb', 'Bob', 'active');
```

**Each test is standalone.** The database has only schema (from prior migrations) and no data. You must insert all FK parents:

```sql
-- FK chain: users → orders → order_items
INSERT INTO users (id, name, email) VALUES (1, 'alice', 'alice@test.com');
INSERT INTO orders (id, user_id, status, created_at)
VALUES (1, 1, 'pending', '2024-01-01 00:00:00');
INSERT INTO order_items (id, order_id, product_name, quantity)
VALUES (1, 1, 'Widget', 3);
```

### test-after-up.sql

Runs AFTER the migration. Asserts the migration did the right thing using `pgroller_test.*` functions.

```sql
SELECT pgroller_test.assert_equal('status backfilled',
    (SELECT status FROM users WHERE name = 'Alice'), 'active');

SELECT pgroller_test.assert_true('email column removed',
    NOT EXISTS(SELECT 1 FROM information_schema.columns
              WHERE table_name='users' AND column_name='email'));

SELECT pgroller_test.assert_count('all users present', 'users', 2);
```

### test-after-down.sql

Runs AFTER the rollback. Asserts the rollback restored state correctly.

```sql
SELECT pgroller_test.assert_equal('status reverted',
    (SELECT status FROM users WHERE name = 'Alice'), NULL);

SELECT pgroller_test.assert_true('email column restored',
    EXISTS(SELECT 1 FROM information_schema.columns
           WHERE table_name='users' AND column_name='email'));
```

---

## Assertion Functions

pgroller injects a `pgroller_test` schema with assertion functions into every test container. Use them in `test-after-up.sql` and `test-after-down.sql`.

| Function | Description |
|----------|-------------|
| `pgroller_test.assert_equal(desc, actual, expected)` | Fails if `actual IS DISTINCT FROM expected` |
| `pgroller_test.assert_true(desc, condition)` | Fails if condition is not `TRUE` |
| `pgroller_test.assert_false(desc, condition)` | Fails if condition is not `FALSE` |
| `pgroller_test.assert_null(desc, value)` | Fails if value is not NULL |
| `pgroller_test.assert_not_null(desc, value)` | Fails if value is NULL |
| `pgroller_test.assert_gt(desc, actual, threshold)` | Fails if actual ≤ threshold |
| `pgroller_test.assert_count(desc, table_name, expected)` | Fails if row count ≠ expected |

All functions raise a PostgreSQL exception on failure, which pgroller catches and reports with the description.

```sql
-- Schema assertions
SELECT pgroller_test.assert_true('index exists',
    EXISTS(SELECT 1 FROM pg_indexes WHERE indexname = 'idx_users_status'));

-- Data assertions
SELECT pgroller_test.assert_equal('count after migration',
    (SELECT count(*) FROM users WHERE status = 'active')::bigint, 5::bigint);

-- Constraint assertions
SELECT pgroller_test.assert_true('check constraint exists',
    EXISTS(SELECT 1 FROM information_schema.table_constraints
           WHERE constraint_name = 'positive_amount'));
```

---

## Reset Strategies

### drop_schema (default, safe)

Each test drops the schema and rebuilds from scratch:

```
For migration N:
  DROP SCHEMA public CASCADE
  CREATE SCHEMA public
  Apply up.sql 0, 1, 2, ..., N-1
  Run test
```

**Trade-off:** O(N²) DDL execution.

### savepoint (fast)

Keeps schema between tests, uses PostgreSQL transactional DDL:

```
Apply baseline once
For migration N:
  BEGIN + SAVEPOINT
  Run test (seed → up → assert → down → assert → diff)
  ROLLBACK TO SAVEPOINT
  Apply up.sql N (advance)
```

**Trade-off:** O(N) execution. Same isolation guarantees.

---

## Workflow

### Adding a New Migration

```bash
# 1. Create the folder
mkdir db/migrations/5__add_audit_log

# 2. Write the migration
cat > db/migrations/5__add_audit_log/up.sql << 'EOF'
CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,
    event VARCHAR(100) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
EOF

# 3. Write the rollback
cat > db/migrations/5__add_audit_log/down.sql << 'EOF'
DROP TABLE audit_log;
EOF

# 4. Test it
pgroller test

# 5. Deploy
pgroller migrate --database "postgresql://..." --accept
```

### Irreversible Migration

```bash
# up.sql
ALTER TABLE users DROP COLUMN legacy_field;

# down.sql — can't restore the data
-- @NoSchemaRollback(column=users.legacy_field, reason="deprecated field, data archived to S3")
```

### Migration with Test Data

```bash
# up.sql
UPDATE users SET status = 'active' WHERE status IS NULL;

# down.sql — can't distinguish original NULLs from backfilled
-- @NoDataRollback(table=users, reason="cannot distinguish original NULL from backfilled values")

# test-before-up.sql — seed a NULL status row
INSERT INTO users (id, name, status) VALUES ('aaa', 'Alice', NULL);

# test-after-up.sql — verify backfill worked
SELECT pgroller_test.assert_equal('backfill', (SELECT status FROM users WHERE name = 'Alice'), 'active');
```

---

## FAQ

### How is this different from Flyway/Liquibase?

Flyway and Liquibase apply migrations. pgroller **proves rollbacks work** before you deploy. They track state; pgroller validates correctness.

### Do I need Docker?

Yes. pgroller uses Testcontainers to spin up real PostgreSQL instances for testing and verification. Your production `migrate`/`rollback`/`status` commands don't need Docker — they connect directly.

### What if my migration is truly irreversible?

Annotate it. `@NoSchemaRollback` or `@NoDataRollback` in `down.sql` explicitly documents what can't be rolled back. pgroller accepts this as a conscious decision and the test passes.

### What if I have 200 existing Flyway migrations?

1. Apply them to a local Postgres
2. `pgroller init --from-database "postgresql://localhost/mydb"`
3. This creates a baseline from the final schema
4. New migrations from this point use pgroller format

### Can I use pgroller alongside Flyway?

Not recommended. Use one migration tool per database. Migrate fully by using `pgroller init --from-database` to snapshot your current state, then switch.

### What's the `pgroller_history` table?

pgroller creates it when you first run `migrate`. It tracks which migrations are applied, with checksums to detect if someone modified an already-deployed migration file.

### Why does rollback verify by default?

Because after rolling back, you want immediate confirmation the schema is in the expected state. A rollback that silently leaves garbage is worse than one that fails loudly.

