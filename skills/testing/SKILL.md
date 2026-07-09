<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Testing Skill

Use when:

* writing or modifying any test
* implementing any feature or behavioral change (every feature needs tests)
* a test is failing

## Required — Test Change Policy

* A failing test is an implementation error signal, not a test error signal; investigate and fix the implementation first.
* If a test failure conflicts with task instructions, stop immediately and ask a human for clarification before proceeding.
* Do not change, delete, or weaken tests to make CI pass without explicit human approval.
* Changing test assertions requires human approval when the goal is unclear or test expectations conflict with task requirements.
* New features must include tests, and test coverage must not decrease.
* Every new feature **must** have at least one Playwright e2e test covering the happy path. In addition, unit tests for both frontend (Vitest) and backend (Rust `#[cfg(test)]`) must be added to cover edge cases and non-trivial logic.
* If a legitimate refactor changes observable behavior, update the tests and explain why in the commit message.
* When in doubt, stop and ask; never silently "fix" a test to unblock work.
* All CI jobs MUST pass. Never exclude or skip tests to fix CI — fix the underlying issue or provide the required infrastructure (e.g. a PostgreSQL service; see `skills/database/SKILL.md`) instead.

## E2E Tests

Playwright e2e tests live in `frontend/e2e/` and run against the demo environment (`http://localhost:8080`).

### Run locally

Start the demo first (see `skills/documentation/SKILL.md` for the demo environment), then:

```bash
cd frontend
npm run e2e
```

### Run with coverage

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

In CI the e2e job runs these steps automatically and uploads the LCOV to Coveralls as the `e2e` flag, which is merged with the `unit` flag (Rust + Vitest) by the `coveralls-finish` job.

## Validation checklist

* [ ] Any test failure was diagnosed as an implementation bug, not "fixed" by loosening the test
* [ ] No test assertion changed without human approval, where required
* [ ] New feature has a Playwright e2e happy-path test
* [ ] New feature has unit tests (Vitest and/or Rust `#[cfg(test)]`) covering edge cases
* [ ] Coverage has not decreased
* [ ] All CI jobs pass, with no tests skipped or excluded
