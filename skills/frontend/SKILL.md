<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Frontend Skill

Use when:

* modifying any `frontend/**/*.{vue,ts,tsx,js,css}` file
* adding or upgrading a frontend dependency

## Required

* All functions must have explicit return type annotations.
* Never use `any` — use proper types or `unknown` with narrowing.
* Use `type` imports for type-only values (`import type { Foo } from '...'`).
* No `console.log` in production code (use sparingly; the linter warns).
* No `debugger` statements.
* Vue templates must use single attribute per line for elements with multiple attributes.
* Never compare a wide `string`-typed value to a literal for control flow (`===`, `!==`, `switch`) — see the `local/no-string-literal-control-flow` ESLint rule below.
* Never add an entry to `frontend/.npm-audit-allowlist.json` without explicit human approval. If CI fails due to a new advisory or a deprecated package, report it to the user and suggest a fix (upgrade or replacement) — do not suppress it.
* **Build verification is MANDATORY.** After ANY change to frontend code, run `npm run build` in `frontend/` and confirm it exits successfully before considering the task complete. A broken production build is never acceptable.
* **Never submit template expressions that contain syntax errors.** Common mistakes: unbalanced quotes in attribute bindings, invalid JavaScript in `v-if`/`v-for`/`:prop` expressions, missing commas in object literals inside templates. If unsure, run `npm run build` to verify.
* Write or update unit/component tests for any non-trivial frontend logic change.

## Workflow

1. Format: `npm run format:check` (verify) / `npm run format` (auto-fix) — Prettier.
2. Lint: `npm run lint` (verify) / `npm run lint:fix` (auto-fix) — ESLint. CI runs with `--max-warnings=0`, so any warning fails CI, not just errors.
3. Build: `npm run build` — mandatory after any change, not optional.
4. Test: `npm run test` (if available), or at minimum `npm run build`, to validate.

For end-to-end tests (Playwright, `frontend/e2e/`), see `skills/testing/SKILL.md`.

## Validation checklist

* [ ] `npm run format:check` clean
* [ ] `npm run lint` clean (zero warnings)
* [ ] `npm run build` succeeds
* [ ] `npm run test` passes (or build validated if no test target applies)
* [ ] No `.npm-audit-allowlist.json` entries added without human approval
* [ ] Non-trivial logic changes have unit/component test coverage

## `local/no-string-literal-control-flow` ESLint rule

The "no string comparisons for control flow" rule is enforced for the frontend by a type-aware custom ESLint rule at `frontend/eslint-rules/no-string-literal-control-flow.js`, wired into `frontend/eslint.config.js` as `local/no-string-literal-control-flow`. Unlike a syntax-only `no-restricted-syntax` rule, it uses the TypeScript checker (via type-aware parsing, `parserOptions.projectService`) to flag a comparison or `switch` only when the non-literal operand's type is the *wide* `string` — not when it's already a narrow string-literal union/enum being compared to one of its own members. That distinction matters because TypeScript's idiomatic "enum" (a `type Foo = 'a' | 'b'` union) is itself expressed via string literals, so a syntax-only rule can't tell a real violation from already-correct code.

* Exempt inside functions with a TS type-predicate return type (`function isFoo(x): x is Foo`) or a return type that is itself a narrow string-literal union — the direct equivalent of Rust's `from`/`from_str`/`try_from`/`deserialize` exemption.
* Also exempt for `typeof` checks and `KeyboardEvent.key` comparisons (a DOM API contract, like Rust's `tracing::field::Field::name()` case), and for comparisons against the empty string literal (presence checks, not domain state).
* A few remaining sites are legitimately string-based (env var/localStorage boolean flags, `window.location.protocol`, PrimeVue's own untyped prop contracts) and carry a `// eslint-disable-next-line local/no-string-literal-control-flow -- reason` comment.
* Backend response fields the Rust side serializes as plain strings (e.g. `status: string` on report/activity rows) are normalized once via shared helpers like `frontend/src/utils/backupStatus.ts` rather than compared ad hoc at each call site.
* Run it locally with `npm run lint` in `frontend/`; it runs alongside the existing ESLint rules, no separate command needed.
