<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Instructions

## Build & Lint

* Run `cargo +nightly fmt -- --config error_on_unformatted=true,error_on_line_overflow=true,format_strings=true,group_imports=StdExternalCrate,imports_granularity=Crate` to format the code.
* Run `cargo +nightly clippy --workspace -- -D warnings` to check for common mistakes.
* Run `cargo test --workspace` to run all tests.
* Clippy pedantic is enforced at workspace level (`deny`). Do not disable or suppress clippy warnings.
* Additional restriction lints are denied workspace-wide: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr` (all allowed in test code via `clippy.toml`), and `allow_attributes_without_reason` (every `#[allow]` must carry a `reason = "..."` — and still requires human approval per the rules below).
* Run `cargo deny check` to audit Rust dependencies (RUSTSEC advisories, yanked versions, licenses, duplicate versions). Configuration lives in `deny.toml`; CI runs this in the `deps-audit` job. **Never add entries to the `ignore` list in `deny.toml`** — only a human may suppress an advisory, same as the npm audit allowlist.
* Frontend lint runs with `--max-warnings=0`: ESLint warnings fail CI, not just errors.

## Pre-commit

* Run `uv run pre-commit run --all-files --show-diff-on-failure` before committing. All hooks MUST pass.
* This validates formatting, linting, license headers, secrets detection, shell scripts, markdown, and more.
* If hooks modify files (e.g., trailing whitespace fixes), stage the changes and re-run until clean.

## CI Requirements

* All CI jobs MUST pass. Never exclude or skip tests to fix CI — fix the underlying issue or provide the required infrastructure (e.g., PostgreSQL service) instead.
* Integration tests MUST always run in CI. Any job that executes tests (`cargo test`, `cargo llvm-cov`, etc.) against code with database dependencies MUST have a PostgreSQL service container configured.
* Never use `--lib --bins` or other flags to avoid running integration tests in CI.

## Database Integration Tests

The test file `crates/server/tests/db_queries.rs` runs against a real PostgreSQL instance. These tests use `#[sqlx::test(migrations = "./migrations")]` which automatically creates and drops isolated databases per test.

### Requirements

* A PostgreSQL server must be running and accessible.
* Set `DATABASE_URL` to a valid connection string with a superuser or a user that has `CREATEDB` privilege (sqlx creates temporary databases per test).
* Example: `DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo +nightly test -p server --test db_queries`

### Local development (Docker)

```bash
docker run -d --name borg-postgres \
  -e POSTGRES_USER=borg \
  -e POSTGRES_PASSWORD=borg_dev \
  -e POSTGRES_DB=borg \
  -p 5432:5432 \
  postgres:latest
```

Then run:

```bash
DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo +nightly test -p server --test db_queries
```

### CI

The GitHub Actions workflow (`.github/workflows/ci.yml`) has a `db-integration` job that spins up a PostgreSQL service container and runs both `db_queries` and the ignored `integration` tests automatically.

## sqlx Offline Cache

The project uses `sqlx` compile-time query macros (`query!`, `query_as!`, `query_scalar!`). These require an offline cache (`.sqlx/` directory) for builds without a live PostgreSQL connection.

### Regenerating the offline cache

After changing any SQL query or running new migrations:

```bash
# Ensure PostgreSQL is running and migrations are applied
DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo sqlx prepare --workspace
```

This regenerates the `.sqlx/` JSON cache files. Commit the updated `.sqlx/` directory alongside SQL changes.

### CI check

CI runs `cargo sqlx prepare --check --workspace` to verify the cache is up to date with the current queries.

CI also runs `scripts/check-no-raw-sqlx-queries.sh`, which fails the build if any Rust source uses sqlx's runtime (non-compile-time-checked) query constructors (`sqlx::query(`, `sqlx::query_as(`, `sqlx::query_as::<...>`, `sqlx::query_scalar(`). Always use `query!`/`query_as!`/`query_scalar!` instead, so a schema change that isn't reflected in the Rust types fails the build. The same check runs locally via the `no-raw-sqlx-queries` pre-commit hook.

### Verifying cache freshness locally

```bash
DATABASE_URL=postgres://borg:borg_dev@localhost:5432/borg cargo sqlx prepare --check --workspace
```

Exits with non-zero status if the cache is stale.

### Writing new DB tests

* Add tests to `crates/server/tests/db_queries.rs`.
* Use `#[sqlx::test(migrations = "./migrations")]` — each test gets a fresh database with all migrations applied.
* Do not rely on specific auto-increment IDs; migrations seed roles (`admin`, `operator`, `viewer`), so use unique names for test data to avoid conflicts.
* The pool argument (`PgPool`) is provided automatically by the macro.

## Frontend Dependency Health

* CI checks for npm security vulnerabilities (`npm audit --audit-level=moderate`) and deprecated packages.
* Allowlists are maintained in `frontend/.npm-audit-allowlist.json`.
* **NEVER add entries to `.npm-audit-allowlist.json`.** Only a human may allowlist a vulnerability or deprecated package. If CI fails due to a new advisory or deprecation, report it to the user and suggest the fix (upgrade or replacement) — do not suppress it.

## E2E Tests

Playwright e2e tests live in `frontend/e2e/` and run against the demo environment (`http://localhost:8080`).

### Run e2e tests locally

Start the demo first (see the Demo Environment section below), then:

```bash
cd frontend
npm run e2e
```

### Run e2e tests with coverage

The e2e tests are instrumented with Istanbul when `VITE_COVERAGE=true`. Coverage JSON files accumulate in `frontend/.nyc_output/` during the run; `nyc report` converts them to LCOV.

```bash
# 1. Build an instrumented frontend
cd frontend
VITE_COVERAGE=true npm run build

# 2. Start the demo, mounting the instrumented build over the container's static dir
cd ..
docker compose \
  -f .devcontainer/demo/docker-compose.demo.yml \
  -f .devcontainer/demo/docker-compose.coverage-override.yml \
  up -d

# 3. Run tests (coverage JSON files land in frontend/.nyc_output/)
cd frontend
VITE_COVERAGE=true npm run e2e

# 4. Generate LCOV
npm run e2e:coverage   # writes frontend/coverage-e2e/lcov.info
```

In CI the e2e job runs these steps automatically and uploads the LCOV to Coveralls
as the `e2e` flag, which is merged with the `unit` flag (Rust + Vitest) by the
`coveralls-finish` job.

## Frontend Lint & Format

* Run `npm run format:check` (in `frontend/`) to verify formatting. Run `npm run format` to auto-fix.
* Run `npm run lint` (in `frontend/`) to check for lint errors. Run `npm run lint:fix` to auto-fix.
* Prettier is the formatter. ESLint handles code quality rules. Both must pass in CI.
* All functions must have explicit return type annotations.
* Never use `any` — use proper types or `unknown` with narrowing.
* Use `type` imports for type-only values (`import type { Foo } from '...'`).
* No `console.log` in production code (use sparingly, warned by linter).
* No `debugger` statements.
* Vue templates must use single attribute per line for elements with multiple attributes.
* **Build verification is MANDATORY.** After ANY change to frontend code (`.vue`, `.ts`, `.tsx`, `.js`, `.css` files in `frontend/`), run `npm run build` in `frontend/` and confirm it exits successfully. A broken production build is never acceptable — fix all errors before considering the task complete.
* **Never submit template expressions that contain syntax errors.** Common mistakes: unbalanced quotes in attribute bindings, invalid JavaScript in `v-if`/`v-for`/`:prop` expressions, missing commas in object literals inside templates. If unsure, run `npm run build` to verify.
* Write or update unit/component tests for any non-trivial frontend logic change. Run `npm run test` (if available) or at minimum `npm run build` to validate.

## Project Structure

* This is a Cargo workspace with three crates: `crates/server`, `crates/agent`, `crates/shared`.
* `crates/shared` contains domain types, the WebSocket protocol schema, and crypto utilities. Both server and agent depend on it.
* `crates/server` is the axum-based HTTP + WebSocket server that serves the Vue.js frontend and provides the REST API.
* `crates/agent` is the client binary that runs on each backup machine, connects to the server, and executes borg commands.
* `frontend/` contains the Vue.js 3 + Vite SPA (TypeScript).
* Modules should mirror the logical architecture. Group related functionality into sub-modules. If a file exceeds ~300 lines or contains multiple logical units, split it.
* Adding a dependency is preferred over shelling out to external commands. Shell out is a last resort and only to be used when there is no library alternative. Still, clarify new dependencies with the user before adding them.

## Security

* Passphrases are encrypted at rest using AES-256-GCM. Never store, log, or transmit passphrases in plaintext.
* Agent tokens must be cryptographically random (32+ bytes).
* Never log sensitive data (passphrases, tokens, SSH keys). Use `[REDACTED]` placeholders in debug output.
* All user-facing input must be validated. Never trust input from agents or API callers without validation.

## Type Safety

* Use strongly typed logic everywhere. String-based logic is forbidden.
* Never use `unwrap()`, `expect()`, or `panic!()` in production code. Always handle errors gracefully with `Result` and the `?` operator.
* `unwrap()` is permitted only in `#[cfg(test)]` code.
* Avoid `unsafe` code.
* Do not use string comparisons for control flow. Use enums or structs instead.
* Prefer newtypes for domain identifiers (e.g., `struct MachineId(i64)`, `struct AgentToken(String)`).

### Enforcement: `no_string_control_flow` lint

The "no string comparisons for control flow" rule is enforced in CI by a custom [dylint](https://github.com/trailofrust/dylint) lint at `lints/no_string_control_flow/`. It denies `==`/`!=` comparisons and `match` arms that test a `&str`/`String` value against a string literal.

* It is exempt inside `from`, `from_str`, `try_from`, and `deserialize` functions — the sanctioned boundary where a raw string is parsed into an enum. Everywhere else, parse into an enum first and branch on that.
* A small number of sites are legitimately string-based (env var key lookups, third-party API contracts like `tracing::field::Field::name()` or `url::Url::scheme()`, or literal path/text-format tokens like borg's `"."` or a systemd `"[Service]"` header). These carry a paired `#[allow(unknown_lints, reason = "...")]` + `#[allow(no_string_control_flow, reason = "...")]`, since `no_string_control_flow` is unknown to plain `rustc`/`clippy` outside the dylint driver.
* Run it locally with:
  ```bash
  rustup toolchain install "$(grep '^channel' lints/no_string_control_flow/rust-toolchain | cut -d'"' -f2)" --profile minimal --component rustc-dev --component llvm-tools-preview
  cargo install cargo-dylint dylint-link --locked
  RUSTFLAGS="-D no_string_control_flow" cargo dylint --all --workspace
  ```
* After changing the lint itself, update its UI test fixtures (`lints/no_string_control_flow/ui/`) and re-run `cargo test` inside the lint crate.
* The lint crate is pinned to its own nightly toolchain (`lints/no_string_control_flow/rust-toolchain`), independent of the workspace's `+nightly`. Bump it with `cargo dylint upgrade lints/no_string_control_flow` when it stops building against current dependencies, and update the CI step's `cargo-dylint`/`dylint-link` install version if the `dylint_linting` major version changes.

### Frontend equivalent: `local/no-string-literal-control-flow` ESLint rule

The same rule is enforced for the frontend by a type-aware custom ESLint rule at `frontend/eslint-rules/no-string-literal-control-flow.js`, wired into `frontend/eslint.config.js` as `local/no-string-literal-control-flow`. Unlike a syntax-only `no-restricted-syntax` rule, it uses the TypeScript checker (via type-aware parsing, `parserOptions.projectService`) to flag a comparison or `switch` only when the non-literal operand's type is the *wide* `string` — not when it's already a narrow string-literal union/enum being compared to one of its own members. That distinction matters because TypeScript's idiomatic "enum" (a `type Foo = 'a' | 'b'` union) is itself expressed via string literals, so a syntax-only rule can't tell a real violation from already-correct code.

* Exempt inside functions with a TS type-predicate return type (`function isFoo(x): x is Foo`) or a return type that is itself a narrow string-literal union — the direct equivalent of Rust's `from`/`from_str`/`try_from`/`deserialize` exemption.
* Also exempt for `typeof` checks and `KeyboardEvent.key` comparisons (a DOM API contract, like Rust's `tracing::field::Field::name()` case), and for comparisons against the empty string literal (presence checks, not domain state).
* A few remaining sites are legitimately string-based (env var/localStorage boolean flags, `window.location.protocol`, PrimeVue's own untyped prop contracts) and carry a `// eslint-disable-next-line local/no-string-literal-control-flow -- reason` comment.
* Backend response fields the Rust side serializes as plain strings (e.g. `status: string` on report/activity rows) are normalized once via shared helpers like `frontend/src/utils/backupStatus.ts` rather than compared ad hoc at each call site.
* Run it locally with `npm run lint` in `frontend/`; it runs alongside the existing ESLint rules, no separate command needed.

## Style Guide

### Prefer `map_or`

```rust
// DO
let s = Some("test").map_or("default".to_string(), |s| s.to_uppercase());

// DON'T
let s = Some("test").map(|s| s.to_uppercase()).unwrap_or("default".to_string());
```

```rust
// DO
let s = Some("test").map_or(<_>::default(), |s| s.to_uppercase());

// DON'T
let s = Some("test").map(|s| s.to_uppercase()).unwrap_or_default();
```

### Control Flow: Use Iterator Chains, Not for Loops

```rust
// DO
let results: Vec<_> = items
    .iter()
    .filter(|item| item.is_valid())
    .map(|item| item.process())
    .collect();

// DON'T
let mut results = Vec::new();
for item in items {
    if item.is_valid() {
        results.push(item.process());
    }
}
```

```rust
// DO
let total: i64 = values.iter().map(|v| v.amount()).sum();

// DON'T
let mut total = 0;
for value in values {
    total += value.amount();
}
```

### Error Handling: Use `?` Operator

```rust
// DO
fn read_file(path: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}

// DON'T
fn read_file(path: &str) -> String {
    std::fs::read_to_string(path).expect("Failed to read file")
}
```

### Early Returns: Use `let ... else`

```rust
// DO
let Some(user) = get_user(id) else {
    return Err(Error::NotFound);
};
let Ok(session) = user.active_session() else {
    return Err(Error::NoSession);
};

// DON'T
if let Some(user) = get_user(id) {
    if let Ok(session) = user.active_session() {
        // deeply nested code
    } else {
        return Err(Error::NoSession);
    }
} else {
    return Err(Error::NotFound);
}
```

```rust
// DO
let Some(value) = maybe_value else { continue };
let Ok(parsed) = input.parse::<i32>() else { continue };

// DON'T
if let Some(value) = maybe_value {
    if let Ok(parsed) = input.parse::<i32>() {
        // ...
    }
}
```

### Variable Naming: Shadow, Don't Rename

```rust
// DO
let input = get_raw_input();
let input = input.trim();
let input = input.to_lowercase();
let input = parse(input)?;

// DON'T
let raw_input = get_raw_input();
let trimmed_input = raw_input.trim();
let lowercase_input = trimmed_input.to_lowercase();
let parsed_input = parse(lowercase_input)?;
```

### Comments

* Keep to a minimum, no obvious comments.
* Good code should be self-explanatory.
* If using comments, explain the "why" behind a decision, not the "what".
* Do not use banner-style comments (lines of `====`, `----`, `////`, etc.) to separate sections. Use modules instead.

### Pattern Matching: Never Use Wildcard Matches

```rust
// DO
match status {
    Status::Pending => handle_pending(),
    Status::Active => handle_active(),
    Status::Completed => handle_completed(),
}

// DON'T
match status {
    Status::Pending => handle_pending(),
    _ => handle_other(),
}
```

If a wildcard genuinely makes sense, ask the user for approval.

### Ownership: Borrow Instead of Clone

Prefer borrowing (`&T`, `&mut T`) over cloning. Only clone when ownership transfer is truly needed.

```rust
// DO
fn process(data: &str) -> Result<()> {
    println!("{data}");
    Ok(())
}

// DON'T
fn process(data: String) -> Result<()> {
    println!("{data}");
    Ok(())
}
```

### Conversions: Use `From`/`Into` Traits

```rust
// DO
impl From<RawConfig> for AppConfig {
    fn from(raw: RawConfig) -> Self {
        Self {
            name: raw.name,
            timeout: Duration::from_secs(raw.timeout_secs),
        }
    }
}
let config: AppConfig = raw_config.into();

// DON'T
impl RawConfig {
    fn to_app_config(&self) -> AppConfig { /* ... */ }
}
```

### Display: Implement `Display`, Not Custom `to_string()` Methods

```rust
// DO
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Pending => write!(f, "pending"),
            Status::Active => write!(f, "active"),
            Status::Completed => write!(f, "completed"),
        }
    }
}

// DON'T
impl Status {
    fn to_string(&self) -> String { /* ... */ }
}
```

### Strings: Use Format Strings, Not Concatenation

```rust
// DO
let msg = format!("{name} has {count} items");
println!("Processing {path:?}");

// DON'T
let msg = name.to_string() + " has " + &count.to_string() + " items";
println!("Processing {:?}", path);
```

### Constructors: Use `Default` and Builder Patterns

```rust
// DO
#[derive(Default)]
struct Config {
    retries: u32,
    verbose: bool,
    timeout: Option<Duration>,
}

let config = Config {
    retries: 3,
    ..Config::default()
};

// DON'T
let config = Config {
    retries: 3,
    verbose: false,
    timeout: None,
};
```

### Derive: Use Derive Macros Over Manual Implementations

Derive standard traits instead of implementing them manually when the default derivation is correct.

```rust
// DO
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Point {
    x: i32,
    y: i32,
}
```

### Newtype Pattern: Wrap Primitive Types for Semantic Meaning

```rust
// DO
struct UserId(u64);
struct Email(String);

fn send_email(to: &Email, from: &Email) { /* ... */ }

// DON'T
fn send_email(to: &str, from: &str) { /* ... */ }
```

### Closures: Prefer Closures Over Named Functions for Short Logic

```rust
// DO
items.iter().filter(|i| i.is_active()).count()

// DON'T
fn is_active(item: &&Item) -> bool { item.is_active() }
items.iter().filter(is_active).count()
```

### Option/Result Combinators: Use `map`, `and_then`, `unwrap_or_else`

```rust
// DO
let name = user
    .nickname()
    .or_else(|| user.full_name())
    .unwrap_or_else(|| "anonymous".to_string());

// DON'T
let name = if let Some(n) = user.nickname() {
    n
} else if let Some(n) = user.full_name() {
    n
} else {
    "anonymous".to_string()
};
```

```rust
// DO
let port = config.port.unwrap_or(8080);

// DON'T
let port = match config.port {
    Some(p) => p,
    None => 8080,
};
```

### Slices: Accept `&[T]` and `&str`, Not `&Vec<T>` and `&String`

```rust
// DO
fn process(items: &[Item]) { /* ... */ }
fn greet(name: &str) { /* ... */ }

// DON'T
fn process(items: &Vec<Item>) { /* ... */ }
fn greet(name: &String) { /* ... */ }
```

### Enums: Use Enums With Data Over Separate Structs

```rust
// DO
enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

fn area(shape: &Shape) -> f64 {
    match shape {
        Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
        Shape::Rectangle { width, height } => width * height,
    }
}
```

### Iterators: Prefer `iter()` Method Chains Over Index Access

```rust
// DO
for (i, item) in items.iter().enumerate() {
    println!("{i}: {item}");
}

// DON'T
for i in 0..items.len() {
    println!("{}: {}", i, items[i]);
}
```

### Tests: Use `#[cfg(test)]` Module in the Same File

```rust
// DO — tests at the bottom of the same file
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_input() {
        let result = parse("42");
        assert_eq!(result, Ok(42));
    }
}
```

## Code Navigation: Always Use rust-analyzer LSP

When searching or navigating Rust code, always use the LSP tools:

* `lsp_goto_definition` — Find where a symbol is defined
* `lsp_find_references` — Find all references to a symbol
* `lsp_symbols` — Get all symbols in a file or workspace
* `lsp_diagnostics` — Check for errors/warnings before building

## Async IO

* **Never use manual `loop { read(); write(); }` patterns for bidirectional IO.** Use two independent async tasks joined with `tokio::join!` — one per direction.
* **Never call blocking IO inside async tasks.** Blocking calls (`std::fs::read_to_string`, `std::fs::write`, `std::thread::sleep`, etc.) must be wrapped in `tokio::task::spawn_blocking`.
* For byte-stream proxying between two `AsyncRead + AsyncWrite` endpoints use `tokio::io::copy_bidirectional`. When one endpoint is frame-based (e.g. WebSocket), use two directional tasks instead.
* Use `tokio::fs` for all async file operations.
* Use `tokio::time::sleep` — never `std::thread::sleep` — in async contexts.

```rust
// DO — two directional tasks
let a_to_b = async { while let Some(data) = stream_a.next().await { sink_b.send(data).await?; } };
let b_to_a = async { while let Some(data) = stream_b.next().await { sink_a.send(data).await?; } };
tokio::join!(a_to_b, b_to_a);

// DO — blocking work off the async executor
tokio::task::spawn_blocking(move || std::fs::write(path, data)).await??;

// DON'T — manual select loop for bidirectional IO
loop {
    tokio::select! {
        data = stream_a.next() => { sink_b.send(data).await; }
        data = stream_b.next() => { sink_a.send(data).await; }
    }
}

// DON'T — blocking IO in async context
async fn save(path: &Path, data: &str) {
    std::fs::write(path, data).unwrap(); // blocks the executor thread
}
```

## SSH Agent Forwarding

Agents can tunnel the server's SSH agent socket to `borg` for passwordless repository access. This avoids distributing SSH keys to agent machines.

### How it works

1. The server exposes a WebSocket endpoint at `/ws/ssh-agent/:hostname`; the agent sends its token as the first WebSocket message.
2. On each backup, the agent creates a temporary Unix domain socket (`$TMPDIR/assimilate-XXXX/agent.sock`).
3. For each connection from borg/ssh to that socket, the agent opens a new WebSocket to the server relay endpoint and pipes bytes bidirectionally.
4. The server connects to its own `SSH_AUTH_SOCK` and relays SSH agent protocol between the WebSocket and the local agent socket.
5. `SSH_AUTH_SOCK` is injected into the borg subprocess environment, giving it transparent access to the server's keys.

### Server setup

Load your borg SSH private key into an ssh-agent on the server and ensure `SSH_AUTH_SOCK` is set in the server's environment:

```bash
eval $(ssh-agent)
ssh-add /path/to/borg_ed25519_key
```

For systemd services, forward the socket via `SSH_AUTH_SOCK` in the unit's `Environment=` or `EnvironmentFile=`:

```ini
[Service]
Environment=SSH_AUTH_SOCK=/run/user/1000/gnupg/S.gpg-agent.ssh
```

For Docker, the container manages its own SSH key pair — no host agent or key mounting required:

```yaml
services:
  server:
    volumes:
      - ssh_keys:/app/ssh
volumes:
  ssh_keys:
```

On first start the container generates an Ed25519 key pair in the `ssh_keys` volume and loads it into a container-local ssh-agent. The public key is printed to container logs at startup and is visible in the admin UI under **System**. Add it to `~/.ssh/authorized_keys` on the borg repository host.

### Agent setup

No configuration required. The agent automatically attempts SSH forwarding before each backup using the same `BORG_SERVER_URL` and `BORG_AGENT_TOKEN` it uses for the main WebSocket connection.

If the server's `SSH_AUTH_SOCK` is not set or unreachable, the backup proceeds without forwarding — SSH authentication falls back to whatever keys are available on the agent machine.

### Borg repository authorization

The borg repository server (typically accessed via `ssh://user@host/path`) must authorize the SSH key loaded in the server's ssh-agent. Add the server's public key to `~/.ssh/authorized_keys` on the borg server host (or use `borg serve --append-only` with a `command=` restriction).

## Documentation

Every user-facing feature or behavioral change must be accompanied by documentation **and** a corresponding update to the demo environment (`.devcontainer/demo/seed-demo.sh`) so the new scenario is included and screenshots can be captured. If the feature maps to an existing page, update that page. If it introduces a new concept or changes the visual appearance of a page, create a new page and add it to `nav:` in `mkdocs.yml`.

### MkDocs Setup

Install dependencies and serve locally:

```bash
pip install -r docs/requirements.txt
mkdocs serve
```

Verify before committing:

```bash
mkdocs build --strict
```

Strict mode catches broken links and missing pages. Always run it before committing doc changes.

### Docs Directory Structure

* `docs/` — all source Markdown files
* `docs_html/` — build output (gitignored; generated by `mkdocs build`)
* `mkdocs.yml` — site configuration at the repo root

### Writing Docs

* Follow the style guide at `docs/contributing/style-guide.md`.
* Use Mermaid fenced blocks for diagrams: ` ```mermaid `.
* Use admonitions for callouts: `!!! note`, `!!! warning`, `!!! tip`.

### Screenshots

Every documentation page that describes a UI feature **must** include a screenshot. Screenshots are stored in `docs/assets/screenshots/` as PNG files at 1280×800 viewport resolution.

* When adding or changing a UI feature, capture a fresh screenshot and save it to `docs/assets/screenshots/<name>.png`.
* Reference screenshots in Markdown with `![Alt text](assets/screenshots/<name>.png)`.
* Place the screenshot immediately after the introductory paragraph of the section it illustrates.
* If a feature change visually affects an existing screenshot, recapture it.
* Screenshot file names use lowercase kebab-case matching the page or view name (e.g., `schedule-detail.png`, `repo-detail.png`, `host-detail.png`).

### Nav

All new pages must be added to the `nav:` section in `mkdocs.yml`. The file `dependency-manifest.md` is intentionally excluded from nav.

## Demo Environment (Screenshots)

The demo devcontainer at `.devcontainer/demo/` provides a self-contained environment pre-populated with realistic data for capturing documentation screenshots. It covers **all scenarios described in the docs**.

### Running

```bash
.devcontainer/start.sh --demo
```

Or equivalently:

```bash
docker compose -f .devcontainer/demo/docker-compose.demo.yml up --build
```

Open `http://localhost:8080` — login: `admin` / `admin`.

The demo always tears down existing containers and volumes before starting, ensuring a clean state.

### What it sets up

The `seed-demo.sh` script populates every documented scenario:

* **Hosts**: 3 connected agents (`web-server-01`, `db-server-01`, `media-store-01`) with display names, plus 2 unmatched imported placeholder clients (`old-webserver (imported)`, `legacy-db-prod (imported)`)
* **Repositories**: 3 repos with different compression (lz4, zstd), encryption (repokey-blake2), and quotas configured
* **Schedules**: Daily, hourly, and weekly with varying retention policies, rate limits, pre-backup commands, and backup sources
* **Backup reports**: 30 days of daily backups (including 1 warning, 1 failure), 72 hours of hourly DB backups (1 failure), 12 weeks of weekly media backups — all with realistic sizes/durations
* **Hostname aliases**: Glob pattern `web-server-*` on `web-server-01` for the unmatched archive scenario
* **Tags**: Host tags (production, staging) and repo tags (critical, archival)
* **Access control**: 3 users (admin, operator1, viewer1), groups (backend-team, data-team), built-in roles
* **Global excludes**: Standard patterns (node\_modules, \_\_pycache\_\_, etc.)
* **Quotas**: Warn/critical thresholds on server-daily and database-hourly repos
* **System events**: Agent connect/disconnect, backup failures/warnings
* **Audit log**: Repository creation, host registration, schedule creation, login events, quota configuration
* **Archives**: Real borg archives with browsable file trees for archive browsing, diff, and export screenshots
* **Archive tags**: `pre-upgrade` and `weekly-baseline` tags on archives
* **Notifications**: Webhook and email channels with rules for failures, warnings, and agent events
* **SSH tunnels**: A reverse tunnel configured for `media-store-01`

### Maintenance rule

When adding a new user-facing feature or documentation page, update `seed-demo.sh` to include demo data for that feature. The demo environment must always cover all documented scenarios so screenshots can be captured without manual setup.

* Do not disable clippy warnings or prefix parameters with `_` to silence warnings. Fix the underlying issue.
* Do not use `as` casts for numeric conversions. Use `From`/`TryFrom` or explicit conversion methods.
* Do not use `Box<dyn Error>` as a return type. Define concrete error enums with `thiserror`.
* Do not commit code with `todo!()`, `unimplemented!()`, or `dbg!()` macros.
* Do not add `#[allow(...)]` attributes without explicit approval.

## Test Change Policy

* A failing test is an implementation error signal, not a test error signal; investigate and fix the implementation first.
* If a test failure conflicts with task instructions, stop immediately and ask a human for clarification before proceeding.
* Do not change, delete, or weaken tests to make CI pass without explicit human approval.
* Changing test assertions requires human approval when the goal is unclear or test expectations conflict with task requirements.
* New features must include tests, and test coverage must not decrease.
* Every new feature **must** have at least one Playwright e2e test covering the happy path. In addition, unit tests for both frontend (Vitest) and backend (Rust `#[cfg(test)]`) must be added to cover edge cases and non-trivial logic.
* If a legitimate refactor changes observable behavior, update the tests and explain why in the commit message.
* When in doubt, stop and ask; never silently "fix" a test to unblock work.
