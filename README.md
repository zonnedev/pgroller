# pgroller

**Test that your PostgreSQL migrations actually roll back.**

Most migration tools can run your `down.sql`. pgroller checks whether it actually restores the database.

For each migration, pgroller runs a round trip against a real PostgreSQL instance:

```text
snapshot -> up.sql -> down.sql -> snapshot -> diff
```

It compares both schema and data. If the database does not return to its previous state, the test fails.

```text
$ pgroller test

31 migrations: 31 passed (8.05s)
```

## Why pgroller?

A rollback can execute successfully and still be wrong.

For example:

```sql
-- up.sql
ALTER TABLE users ADD COLUMN display_name VARCHAR(100);

UPDATE users
SET display_name = first_name || ' ' || last_name;

ALTER TABLE users DROP COLUMN first_name;
ALTER TABLE users DROP COLUMN last_name;
```

A seemingly valid rollback:

```sql
-- down.sql
ALTER TABLE users ADD COLUMN first_name VARCHAR(100);
ALTER TABLE users ADD COLUMN last_name VARCHAR(100);
ALTER TABLE users DROP COLUMN display_name;
```

The SQL succeeds. The columns are restored.

The data is not.

pgroller catches it:

```text
12__merge_names: FAILED

  Data difference in table: users

  Suggested annotation:
  -- @NoDataRollback(table=users, reason="TODO")
```

## How it works

For every migration, pgroller:

```text
1. Resets the database to version N-1
2. Runs test-before-up.sql (if present)
3. Takes a schema and data snapshot
4. Runs up.sql
5. Runs test-after-up.sql (if present)
6. Runs down.sql
7. Runs test-after-down.sql (if present)
8. Takes another snapshot
9. Diffs the two states
```

Tests run against a disposable PostgreSQL instance using Testcontainers.

`pgroller test` never touches your production database.

## Requirements

- Docker (for Testcontainers)

## Development environment

pgroller includes an optional Nix flake for a reproducible development environment.

If you use [Nix](https://nixos.org/download) and [direnv](https://direnv.net/):

```bash
git clone <repo-url>
cd pgroller
direnv allow
```

The flake provides Rust, Cargo, Clippy, rustfmt, libclang, and the required build dependencies.

```bash
cargo test
cargo build --release
```


## Quick start

Initialize a project:

```bash
pgroller init ./db/migrations
```

Create a migration:

```bash
pgroller new "add status column"
```

Project structure:

```text
db/migrations/
|-- pgroller.toml
|-- 0__baseline/
|   `-- up.sql
`-- 1__add_status_column/
    |-- up.sql
    |-- down.sql
    |-- test-before-up.sql
    |-- test-after-up.sql
    `-- test-after-down.sql
```

`up.sql` and `down.sql` are required for normal migrations. Test files are optional.

### up.sql

```sql
ALTER TABLE users
ADD COLUMN status VARCHAR(50) DEFAULT 'active';
```

### down.sql

```sql
ALTER TABLE users
DROP COLUMN status;
```

Run the rollback test:

```bash
pgroller test
```

```text
1__add_status_column: passed
```

## Data migrations

pgroller also detects data that was not restored by a rollback.

For example:

```sql
-- up.sql
UPDATE users
SET status = 'active'
WHERE status IS NULL;
```

After this runs, there is no way to know which rows originally contained `NULL`.

Use `test-before-up.sql` to provide data that exercises the migration:

```sql
INSERT INTO users (id, name, status) VALUES
    (1, 'Alice', NULL),
    (2, 'Bob', 'active');
```

If `down.sql` does not restore the original state:

```text
3__backfill_status: FAILED

  Data difference in table: users

  Suggested annotation:
  -- @NoDataRollback(table=users, reason="TODO")
```

## Irreversible migrations

Not every migration can be reversed.

If a difference is intentional, document it in `down.sql`.

For schema changes:

```sql
-- up.sql
ALTER TABLE users DROP COLUMN email;
```

```sql
-- down.sql
-- @NoSchemaRollback(column=users.email, reason="email data cannot be reconstructed")
```

For data changes:

```sql
-- @NoDataRollback(table=users, reason="cannot distinguish original NULL values from backfilled values")
```

Annotations do not disable validation. They acknowledge a specific difference that pgroller detected.

If an annotation no longer matches a difference, pgroller reports it as stale:

```text
5__something: warning

  Stale annotation:
  @NoDataRollback(table=users)
```

This usually means either the rollback was fixed or the test data no longer exercises the relevant path.

## Migration tests

Test files are optional and contain normal SQL.

### test-before-up.sql

Runs before the migration. Use it to seed data:

```sql
INSERT INTO users (id, name, status)
VALUES (1, 'Alice', NULL);
```

### test-after-up.sql

Runs after `up.sql`:

```sql
SELECT pgroller_test.assert_equal(
    'status backfilled',
    (SELECT status FROM users WHERE name = 'Alice'),
    'active'
);
```

### test-after-down.sql

Runs after `down.sql`:

```sql
SELECT pgroller_test.assert_null(
    'status restored',
    (SELECT status FROM users WHERE name = 'Alice')
);
```

Available assertions:

```text
assert_equal
assert_true
assert_false
assert_null
assert_not_null
assert_gt
assert_count
```

Test files are only executed by `pgroller test` against disposable databases. They are never executed by `migrate` or `rollback`.

## Commands

### Test migrations

```bash
pgroller test
```

### Create a migration

```bash
pgroller new "add audit log"
```

### Apply migrations

```bash
pgroller migrate --database "postgresql://user:pass@host:5432/db"
```

Preview:

```bash
pgroller migrate \
  --database "postgresql://user:pass@host:5432/db" \
  --dry-run
```

Non-interactive:

```bash
pgroller migrate \
  --database "postgresql://user:pass@host:5432/db" \
  --accept
```

### Roll back

Roll back the latest migration:

```bash
pgroller rollback \
  --database "postgresql://user:pass@host:5432/db"
```

Roll back multiple migrations:

```bash
pgroller rollback \
  --database "postgresql://user:pass@host:5432/db" \
  --steps 3
```

### Status

```bash
pgroller status \
  --database "postgresql://user:pass@host:5432/db"
```

### Verify

Compare a live database against the expected schema:

```bash
pgroller verify \
  --database "postgresql://user:pass@host:5432/db"
```

Clean:

```text
Schema matches expected state (version 31)
```

Drift detected:

```text
Schema drift detected:

  Extra column: users.temp_flag
  Missing index: idx_users_status
```

### Baseline

Collapse the current migration history into a new baseline:

```bash
pgroller baseline
```

Preview:

```bash
pgroller baseline --dry-run
```

## Production safety

When running migrations against a database, pgroller:

- runs each migration in its own transaction
- uses an advisory lock to prevent concurrent migrations
- records applied migrations in `pgroller_history`
- stores checksums to detect modified migrations
- shows irreversible annotations before execution
- verifies the schema after rollback

Only `up.sql` and `down.sql` are executed against the target database.

Test files and seed data stay in the test environment.

## Configuration

`pgroller.toml`:

```toml
[migrations]
dir = "."

[database]
postgres_version = "15"
extensions = []
schema = "public"

[test]
timeout = 30
continue_on_failure = true
reset_strategy = "drop_schema"
```

Reset strategies:

| Strategy | Description | Complexity |
| --- | --- | --- |
| `drop_schema` | Rebuild from previous migrations for every test | O(N^2) |
| `savepoint` | Use PostgreSQL transactions to isolate tests | O(N) |

`drop_schema` is the default.

## Existing databases

Create a baseline from an existing database:

```bash
pgroller init ./db/migrations \
  --from-database "postgresql://localhost/mydb"
```

Or from a `pg_dump`:

```bash
pgroller init ./db/migrations \
  --from-dump ./schema.sql
```

The current schema becomes `0__baseline`. New migrations can then be tested with pgroller.

## Documentation

See [`docs/guide.md`](docs/guide.md) for the full documentation.

## License

MIT

