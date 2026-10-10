<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Alexander Mohr -->

# Contributing

This page covers how to set up a development environment, run tests, and generate coverage reports.

## Prerequisites

- [Rust](https://rustup.rs/): the pinned stable toolchain (1.98) for building, plus a nightly toolchain for formatting and linting
- [Node.js](https://nodejs.org/) 20+
- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- [uv](https://docs.astral.sh/uv/) (Python package manager, for pre-commit)

## Getting started

```bash
# Clone the repo
git clone https://github.com/alexmohr/assimilate
cd assimilate

# Install the pinned stable toolchain used for release builds
rustup toolchain install 1.98

# Install Rust nightly with the components used for formatting and linting
rustup toolchain install nightly
rustup component add rustfmt clippy --toolchain nightly

# Install frontend dependencies
npm ci --prefix frontend

# Install pre-commit hooks
uv run pre-commit install
```

## Running the demo environment

The demo environment provides a fully seeded server for manual testing and documentation screenshots.

```bash
.devcontainer/start.sh --demo
```

Or directly with Docker Compose:

```bash
docker compose -f .devcontainer/demo/docker-compose.demo.yml up --build
```

Open `http://localhost:8080` — login: `admin` / `admin`.

## Build and lint

### Rust

Release artifacts (the agent binaries and the server/agent Docker images) are
built with the pinned stable compiler, so product code must not depend on
nightly-only features. Formatting and linting stay on nightly because the
rustfmt options below are unstable.

When bumping the stable pin, update every place that names it in the same
change. Find them all with:

```bash
git grep -nE '1\.98' -- ':!*Cargo.lock' ':!frontend'
```

At the time of writing that covers the `RUST_STABLE_TOOLCHAIN` variable at the
top of `.github/workflows/ci.yml` (every job in that file reads it from there)
and the matching variable at the top of `.github/workflows/build-agent.yml`
(the reusable release agent build, which does not inherit the caller's env), the
`rust-builder` base image in `Dockerfile.server` and `Dockerfile.agent`, the
`chef` base image in `.devcontainer/demo/Dockerfile.demo`, this page (the
prerequisites, setup and release build commands), the prerequisites and build
command in `docs/getting-started.md`, and the validation checklist in
`skills/rust/SKILL.md`.

```bash
# Release build (matches CI and the Docker images)
cargo +1.98 build --release --locked --workspace

# Format
cargo +nightly fmt -- \
  --config error_on_unformatted=true,error_on_line_overflow=true,\
format_strings=true,group_imports=StdExternalCrate,imports_granularity=Crate

# Lint
cargo +nightly clippy --workspace -- -D warnings

# Unit and integration tests (requires PostgreSQL — see below)
cargo test --workspace
```

### Frontend

```bash
cd frontend

npm run format:check   # Prettier formatting
npm run lint           # ESLint
npm run test           # Vitest unit tests
npm run build          # Production build (must succeed before committing)
```

### Rules implemented on both sides

A few small rules exist in both Rust and TypeScript because the frontend needs them on every keystroke: the file-change pattern grammar, the notification template defaults and placeholder keys, and the hook timeout limit. Their cases live once in `testdata/parity/*.json`, and both the Rust and the Vitest suites assert against them, so changing one side without the other fails CI. When you change one of these rules, update the fixture first.

Anything that doesn't need to run offline in the browser, such as cron validation, next-run previews and notification template rendering, is not duplicated: the frontend asks the server, which answers with the code it actually uses.

## Database integration tests

Tests in `crates/server/tests/db_queries.rs` require a live PostgreSQL instance.

Start one with Docker:

```bash
docker run -d --name borg-postgres \
  -e POSTGRES_USER=borg \
  -e POSTGRES_PASSWORD=borg_dev \
  -e POSTGRES_DB=borg \
  -p 5432:5432 \
  postgres:latest
```

Then run the tests:

```bash
DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg \
  cargo +nightly test -p server --test db_queries
```

## E2E tests

Playwright tests live in `frontend/e2e/` and run against the demo environment.

### Run

Start the demo environment first, then:

```bash
cd frontend
npm run e2e
```

### Run with coverage

Istanbul instrumentation is activated by setting `VITE_COVERAGE=true` at build time. The instrumented bundle writes `window.__coverage__` in the browser; the Playwright fixture captures it after each test and saves JSON files to `frontend/.nyc_output/`.

```bash
# 1. Build with Istanbul instrumentation
cd frontend
VITE_COVERAGE=true npm run build

# 2. Start the demo, mounting the instrumented build over the container's static files
cd ..
docker compose \
  -f .devcontainer/demo/docker-compose.demo.yml \
  -f .devcontainer/demo/docker-compose.coverage-override.yml \
  up -d

# 3. Run tests — coverage JSON files accumulate in frontend/.nyc_output/
cd frontend
VITE_COVERAGE=true npm run e2e

# 4. Generate LCOV report
npm run e2e:coverage   # writes frontend/coverage-e2e/lcov.info
```

!!! note
    The instrumented build is significantly larger than the production build (Istanbul adds counter code to every statement). Use only for coverage measurement, not deployment.

## Code coverage

### Unit coverage (Rust + Vitest)

Rust coverage uses `cargo-llvm-cov`:

```bash
# Install once
cargo install cargo-llvm-cov

DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg \
  cargo +nightly llvm-cov --workspace --lcov --output-path lcov.info \
  -- --include-ignored --test-threads=1
```

Frontend Vitest coverage:

```bash
cd frontend
npm run test:coverage   # writes frontend/coverage/lcov.info
```

### Merging all coverage

To produce a single merged LCOV file (the same way CI does):

```bash
# Rust + Vitest + e2e
sed 's|^SF:|SF:frontend/|' frontend/coverage/lcov.info > frontend-lcov-fixed.info
cat lcov.info frontend-lcov-fixed.info frontend/coverage-e2e/lcov.info > merged.info
```

### CI coverage

In CI, coverage is collected by three jobs and reported to Coveralls:

| Source | Tool | Coveralls flag |
|--------|------|----------------|
| Rust unit + integration tests | `cargo-llvm-cov` | `unit` |
| Frontend Vitest unit tests | `vitest --coverage` | `unit` |
| Playwright e2e tests | `vite-plugin-istanbul` + `nyc` | `e2e` |

The `coveralls-finish` job finalises the report after both jobs complete.

## Pre-commit hooks

```bash
uv run pre-commit run --all-files --show-diff-on-failure
```

All hooks must pass before committing. If a hook modifies files (trailing whitespace, formatting), stage the changes and re-run.
