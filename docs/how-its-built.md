<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# How It's Built

This page describes how Assimilate is developed and which automated gates every change passes before it reaches `main`. Read it to judge whether the project's quality controls fit your risk tolerance.

!!! warning "Alpha software"
    Assimilate is alpha software. Expect breaking changes and data-format migrations between releases. Keep an independent copy of any data you cannot afford to lose.

## AI-assisted development

Coding agents write most of the code, under human direction and review. They work under the rules in [`AGENTS.md`](https://github.com/alexmohr/assimilate/blob/main/AGENTS.md) and the task-specific skills in `skills/`, which cover Rust, frontend, database, security, testing, and documentation work.

The rules include:

- A failing test signals an implementation bug. Tests are never weakened, skipped, or deleted to get CI green without human approval.
- Lint suppressions (`#[allow(...)]`, audit allowlists) need explicit human approval.
- Passphrases, tokens, and SSH keys are never logged or stored in plaintext.
- Every user-facing change ships with documentation and demo data.
- Test coverage must not decrease.

## Automated gates

Every pull request runs these checks in CI:

| Area | Checks |
|------|--------|
| Rust | `rustfmt`, Clippy, a custom lint that forbids string-based control flow, unit and integration tests, tests on a pinned nightly toolchain |
| Database | Integration tests against PostgreSQL, a freshness check for the sqlx offline query cache |
| Frontend | Prettier, ESLint, unit tests, production build, `npm audit`, deprecated-package check |
| End to end | Playwright tests against a seeded server with real agents and borg repositories |
| Contracts | Generated TypeScript types must match the Rust API types |
| Dependencies | `cargo-deny` for advisories, licenses, banned crates, and sources |
| Coverage | Rust and frontend coverage merged and reported to Coveralls; a diff check flags coverage drops |
| Repository hygiene | Duplicate-code detection, REUSE license headers, secret scanning, pre-commit hooks |
| Documentation | `mkdocs build --strict` |

## Strong typing

Rust and the frontend both reject string comparisons as control flow. Values are parsed into enums or narrow union types at the boundary, and the compiler checks every branch. The rule is enforced by the `no_string_control_flow` lint in `lints/` and by frontend lint rules.

## Security posture

- Repository passphrases, TOTP secrets, SMTP passwords, and webhook header values are encrypted at rest with AES-256-GCM.
- Agent tokens are 32 random bytes and stored as bcrypt hashes.
- SSH private keys stay on the server and are lent to agents through an ssh-agent relay.

See [Security & Authentication](security.md) for the full model.

## Related pages

- [Comparison](comparison.md)
- [Architecture](architecture.md)
- [Development](contributing/development.md)
