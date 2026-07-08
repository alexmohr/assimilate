<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Rust Skill

Use when:

* modifying any `.rs` file
* changing `Cargo.toml`, workspace dependencies, or crate/module structure
* touching `lints/no_string_control_flow/`

## Required

* Never use `unwrap()`, `expect()`, or `panic!()` in production code — always handle errors with `Result` and the `?` operator. `unwrap()` is permitted only inside `#[cfg(test)]`.
* Never leave `todo!()`, `unimplemented!()`, or `dbg!()` in committed code.
* Avoid `unsafe` code.
* Never use `as` for numeric conversions — use `From`/`TryFrom` or an explicit conversion method.
* Never return `Box<dyn Error>` — define a concrete error enum with `thiserror`.
* Never disable a clippy warning or prefix an unused parameter with `_` to silence it — fix the underlying issue.
* Never add `#[allow(...)]` without explicit human approval (see `AGENTS.md`); an approved `#[allow]` must still carry `reason = "..."`.
* Never compare a `&str`/`String` to a literal for control flow (`==`, `!=`, `match` arms) — parse into an enum first. Exempt only inside `from`, `from_str`, `try_from`, and `deserialize` functions, the sanctioned boundary where a raw string becomes an enum. Enforced by the `no_string_control_flow` dylint (details below).
* Prefer newtypes for domain identifiers (e.g. `struct MachineId(i64)`, `struct AgentToken(String)`).
* Prefer functional/iterator-chain style over hand-written `for` loops and index access — see Style Guide below. This is not fully enforced by clippy, so follow it deliberately.
* Modules should mirror the logical architecture. Group related functionality into sub-modules. If a file exceeds ~300 lines or contains multiple logical units, split it.
* Adding a dependency is preferred over shelling out to an external command; shelling out is a last resort. Clarify new dependencies with the user before adding them.
* Always use rust-analyzer LSP tools to navigate code: `lsp_goto_definition`, `lsp_find_references`, `lsp_symbols`, `lsp_diagnostics`.

## Workflow

1. Write/modify code following the Style Guide and Async IO rules below.
2. Format: `cargo +nightly fmt -- --config error_on_unformatted=true,error_on_line_overflow=true,format_strings=true,group_imports=StdExternalCrate,imports_granularity=Crate`
3. Lint: `cargo +nightly clippy --workspace -- -D warnings`
4. Test: `cargo test --workspace`
5. Audit dependencies: `cargo deny check` (RUSTSEC advisories, yanked versions, licenses, duplicate versions; config in `deny.toml`). **Never add entries to the `deny.toml` `ignore` list** — only a human may suppress an advisory.

## Validation checklist

* [ ] `cargo +nightly fmt` produces no diff
* [ ] `cargo +nightly clippy --workspace -- -D warnings` is clean. Clippy `pedantic` is `deny` workspace-wide; restriction lints `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`, `allow_attributes_without_reason` are also denied (all permitted in test code via `clippy.toml`)
* [ ] `cargo test --workspace` passes
* [ ] `cargo deny check` passes, with no new `ignore` entries
* [ ] No new `#[allow(...)]` without human approval

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

### Enforcement: `clippy::disallowed_methods`

Blocking `std::fs::*` functions and `std::path::Path` filesystem probes (`exists`, `is_dir`, `is_file`, `metadata`, `read_dir`, `canonicalize`, ...) are denied workspace-wide via `clippy::disallowed_methods` (`Cargo.toml`), with the disallowed path list and per-path rationale in `clippy.toml`.

* Prefer converting the whole call chain to `async fn` using `tokio::fs` (as with `load_server_private_key`/`agent_binary_dir`) over adding a `spawn_blocking` wrapper — `tokio::fs::*` already runs the blocking syscall via `spawn_blocking` internally, so wrapping it again is redundant.
* Sites that are legitimately synchronous — build scripts (`build.rs`, which run at compile time before any async runtime exists), code already inside a `spawn_blocking` closure that mixes CPU-bound work with IO (e.g. the key-generation block in `system.rs`), and test modules — carry a `#[allow(clippy::disallowed_methods, reason = "...")]` at the enclosing fn/module.
* Run it locally with `cargo clippy --workspace --all-targets`; no separate command needed.

## `no_string_control_flow` lint

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
