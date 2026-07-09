<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Database Skill

Use when:

* writing or modifying a SQL query
* adding or changing a migration
* touching `crates/server/tests/db_queries.rs`
* using `sqlx` query macros

## Required

* Always use `query!`, `query_as!`, or `query_scalar!` (compile-time checked). Never use sqlx's runtime constructors (`sqlx::query(`, `sqlx::query_as(`, `sqlx::query_as::<...>`, `sqlx::query_scalar(`) — a schema change not reflected in the Rust types must fail the build, not fail silently at runtime. Enforced by `scripts/check-no-raw-sqlx-queries.sh` in CI and the `no-raw-sqlx-queries` pre-commit hook.
* After changing any SQL query or adding a migration, regenerate the sqlx offline cache and commit the updated `.sqlx/` directory alongside the SQL change.
* Integration tests MUST always run in CI. Any job that runs tests (`cargo test`, `cargo llvm-cov`, etc.) against DB-dependent code MUST have a PostgreSQL service container configured. Never use `--lib --bins` or similar flags to skip integration tests in CI.
* Do not rely on specific auto-increment IDs in tests — migrations seed roles (`admin`, `operator`, `viewer`); use unique names for test data to avoid conflicts.

## Workflow

1. Ensure a PostgreSQL server is running and accessible, with `DATABASE_URL` pointing to a user that has `CREATEDB` privilege (sqlx creates an isolated temporary database per test).

   Local Docker setup:

   ```bash
   docker run -d --name borg-postgres \
     -e POSTGRES_USER=borg \
     -e POSTGRES_PASSWORD=borg_dev \
     -e POSTGRES_DB=borg \
     -p 5432:5432 \
     postgres:latest
   ```

2. Run the DB integration tests:

   ```bash
   DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo +nightly test -p server --test db_queries
   ```

3. After changing SQL or migrations, regenerate the offline cache:

   ```bash
   DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo sqlx prepare --workspace
   ```

4. Verify the cache is fresh before committing:

   ```bash
   DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo sqlx prepare --check --workspace
   ```

   Exits non-zero if the cache is stale. CI runs the same check (`cargo sqlx prepare --check --workspace`).

5. Add new DB tests to `crates/server/tests/db_queries.rs` using `#[sqlx::test(migrations = "./migrations")]` — each test gets a fresh database with all migrations applied; the `PgPool` argument is provided automatically by the macro.

In CI, the `db-integration` job (`.github/workflows/ci.yml`) spins up a PostgreSQL service container and runs both `db_queries` and the ignored `integration` tests automatically.

## Validation checklist

* [ ] Only `query!`/`query_as!`/`query_scalar!` used, no raw sqlx constructors
* [ ] `.sqlx/` regenerated and committed if any query or migration changed
* [ ] `cargo sqlx prepare --check --workspace` passes
* [ ] `db_queries.rs` tests pass against a real PostgreSQL instance
* [ ] New/changed CI test jobs touching DB code have a PostgreSQL service container
* [ ] Test data uses unique names, not assumptions about auto-increment IDs
