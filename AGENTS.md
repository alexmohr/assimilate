<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Agent Instructions

Assimilate is a Cargo workspace (`crates/server`, `crates/agent`, `crates/shared`) plus a Vue 3 + Vite frontend (`frontend/`) for managing borg backups across multiple hosts. `crates/server` is the axum HTTP/WebSocket server; `crates/agent` runs on each backup machine; `crates/shared` holds domain types, the WebSocket protocol schema, and crypto utilities shared by both.

The rules below apply to *every* task. Task-specific instructions live under `skills/` — read the relevant skill before starting work that matches it; each one contains mandatory rules, not just suggestions.

## Non-negotiable rules

* **Tests are truth.** A failing test signals an implementation bug, not a test bug — fix the implementation first. Never change, delete, or weaken a test (or its assertions) to make CI pass without explicit human approval. If a test failure conflicts with your task instructions, stop and ask a human before proceeding.
* **CI must pass in full.** Never exclude, skip, or disable tests to get CI green — fix the underlying issue or provide the missing infrastructure instead.
* **No self-authorized suppressions.** Never add a `#[allow(...)]`, a `deny.toml` `ignore` entry, or a `.npm-audit-allowlist.json` entry without explicit human approval, no matter how minor the violation looks.
* **Secrets stay secret.** Passphrases are encrypted at rest (AES-256-GCM) and must never be stored, logged, or transmitted in plaintext. Agent tokens must be cryptographically random (32+ bytes). Never log sensitive data (passphrases, tokens, SSH keys) — use `[REDACTED]` in debug output. Validate all input from users, agents, and API callers; never trust it implicitly.
* **Strong typing everywhere.** String-based logic is forbidden as a substitute for real types. Never compare a string literal to drive control flow (`match`/`==`/`!=` on a `&str`/`String`) — parse into an enum or a narrow union first. Both Rust and the frontend enforce this with dedicated tooling — see `skills/rust/SKILL.md` and `skills/frontend/SKILL.md`.
* **Don't shell out.** Prefer adding a library dependency over shelling out to an external command; shelling out is a last resort. Always clarify a new dependency with the user before adding it.
* **Every user-facing change needs docs.** A new or changed user-facing feature or behavior must ship with a documentation update and a corresponding update to the demo environment (`.devcontainer/demo/seed-demo.sh`). See `skills/documentation/SKILL.md`.
* **Every new feature needs tests.** Test coverage must never decrease. See `skills/testing/SKILL.md` for the specific requirements (e2e, unit, coverage).

## Workflow

1. Identify which skill(s) below apply to the task and read them before making changes.
2. Make the change.
3. Run the validation commands from the relevant skill(s) (formatting, lint, tests, build).
4. Run `uv run pre-commit run --all-files --show-diff-on-failure`. All hooks MUST pass. If a hook modifies files (e.g. trailing whitespace), stage the changes and re-run until clean.
5. If the task touches tests, re-check `skills/testing/SKILL.md`'s Test Change Policy before touching any assertion.
6. If the task is reviewing or responding to a PR, follow `skills/review/SKILL.md`.

## Skills

| Trigger | Skill |
|---|---|
| Any `.rs` file, `Cargo.toml`, `crates/*`, `lints/*` | `skills/rust/SKILL.md` |
| Any `frontend/**/*.{vue,ts,tsx,js,css}` | `skills/frontend/SKILL.md` |
| SQL queries, migrations, `crates/server/tests/db_queries.rs`, sqlx macros | `skills/database/SKILL.md` |
| Writing/modifying any test, or any feature work (every feature needs tests) | `skills/testing/SKILL.md` |
| Auth, tokens, passphrases, crypto, SSH forwarding, input validation | `skills/security/SKILL.md` |
| New/changed user-facing feature, new docs page | `skills/documentation/SKILL.md` |
| Reviewing a PR, or responding to review comments | `skills/review/SKILL.md` |
