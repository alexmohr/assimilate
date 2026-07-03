# no_string_control_flow

### What it does

Flags `==`/`!=` comparisons and `match` expressions that branch on equality
with a string literal, where the compared value has type `&str` or `String`.

### Why is this bad?

This project's style guide forbids driving control flow off of raw string
comparisons: a typo in the literal silently falls through instead of failing
to compile, and the set of valid values isn't documented anywhere the
compiler can check. Parse the string into an enum at the boundary and match
on that instead.

### Known problems

Only catches direct `==`/`!=` comparisons and `match` patterns against string
literals; it won't catch equivalent logic expressed through method calls
(e.g. `s.eq("foo")`) or `HashMap` lookups.

It is exempt inside `from`, `from_str`, `try_from`, and `deserialize`
functions -- the sanctioned boundary where a raw string is parsed into an
enum.

### Example

```rust
fn handle(status: &str) {
    if status == "active" {
        // ...
    }
}
```

Use instead:

```rust
enum Status {
    Active,
    Inactive,
}

fn handle(status: Status) {
    if status == Status::Active {
        // ...
    }
}
```

See `AGENTS.md`'s "Enforcement: `no_string_control_flow` lint" section for
how to run this locally and how to bump its pinned nightly toolchain.
