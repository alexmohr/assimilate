# Contributing

This page covers how to set up a development environment, run tests, and generate coverage reports.

## Prerequisites

- [Rust](https://rustup.rs/) (nightly toolchain)
- [Node.js](https://nodejs.org/) 20+
- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- [uv](https://docs.astral.sh/uv/) (Python package manager, for pre-commit)

## Getting started

```bash
# Clone the repo
git clone https://github.com/alexmohr/assimilate
cd assimilate

# Install Rust nightly with required components
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

```bash
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
