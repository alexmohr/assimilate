# Dependency Manifest

New dependencies required by the product documentation system.
This manifest was reviewed and approved before any dependencies were added.

## Rust (Cargo.toml — crates/server)

| Crate | Version | License | Justification |
|-------|---------|---------|---------------|
| utoipa | ^4 | MIT/Apache-2.0 | OpenAPI 3.0 spec generation from #[utoipa::path] annotations on axum handlers |
| utoipa-axum | ^0.1 | MIT/Apache-2.0 | axum router integration for utoipa — allows attaching OpenAPI annotations to axum routes |
| utoipa-scalar | ^0.2 | MIT/Apache-2.0 | Serves Scalar interactive API docs UI at /api/docs |

### Compatibility check

- Current axum version: check `crates/server/Cargo.toml` for the `axum` version line
- utoipa 4.x supports axum 0.7+ via utoipa-axum 0.1
- Note if the current version differs

## Python (new toolchain — docs only)

| Package | Version | License | Justification |
|---------|---------|---------|---------------|
| mkdocs-material | >=9.5,<10 | MIT | MkDocs Material theme with built-in Mermaid, search, dark/light mode |

Install: `pip install -r docs/requirements.txt` (venv only, not a cargo/npm dependency)

## Node.js (frontend/package.json devDependency)

| Package | Version | License | Justification |
|---------|---------|---------|---------------|
| @playwright/test | ^1.43 | Apache-2.0 | Automated screenshot capture of all UI pages for embedding in docs |
| tsx | ^4 | MIT | Execute TypeScript files directly (used by seed-data.ts script) |

## What Each Replaces

All dependencies listed above are NEW additions with no existing equivalent in the project.

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
