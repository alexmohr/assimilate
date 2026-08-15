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
* **Use the design tokens.** Never write a literal `font-size`, `border-radius` or transition duration — take it from `frontend/src/style.css`. `frontend/src/design-tokens.test.ts` enforces this and will fail CI on a literal or on a `var()` that references an undefined token. See the "Design tokens" section below.

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

## Design tokens

All visual constants live in `frontend/src/style.css` and are enforced by `frontend/src/design-tokens.test.ts`. The audit that produced them is `docs/contributing/ui-design-audit.md`.

* **Type scale** — `--fs-2xs` `--fs-xs` `--fs-sm` `--fs-base` `--fs-md` `--fs-lg` `--fs-xl`, plus `--fs-2xl` / `--fs-3xl` for decorative numerals only (error codes, score rings). Never a literal `rem` value. The `--fs-*` name is deliberate: Tailwind's theme already owns `--text-*` to drive its `text-*` utilities, so reusing those names would make one token resolve to two different values depending on how it was reached.
* **Radius** — `--radius-sm`, `--radius`, `--radius-pill`. Literal `50%` (a circle) and `0` (a deliberate square edge) are the only exceptions.
* **Duration** — `--duration-fast` (0.1s), `--duration-base` (0.15s, the default for hover/state feedback), `--duration-slow` (0.3s), `--duration-value` (0.4s, for progress fills and chart sweeps). Keyframe `animation` timings are exempt; `transition` is not.
* **Colour** — every colour comes from a token defined in both `:root` and `.dark`. A `var(--x, fallback)` with a hardcoded fallback is a bug: if `--x` exists the fallback is dead, and if it does not the value silently stops following the theme.
* **Focus** — a single `:focus-visible` rule in `@layer base` covers every control. It is wrapped in `:where()` so it carries zero specificity. Do not add per-component focus styling, and do not use `outline: none` without `:focus-visible` handling (the global rule survives `outline: none` on `:focus`, which is why the pattern is safe).
* **Motion** — `@media (prefers-reduced-motion: reduce)` in `@layer base` neutralises every animation and transition. Do not add motion that bypasses it.

## `local/no-string-literal-control-flow` ESLint rule

The "no string comparisons for control flow" rule is enforced for the frontend by a type-aware custom ESLint rule at `frontend/eslint-rules/no-string-literal-control-flow.js`, wired into `frontend/eslint.config.js` as `local/no-string-literal-control-flow`. Unlike a syntax-only `no-restricted-syntax` rule, it uses the TypeScript checker (via type-aware parsing, `parserOptions.projectService`) to flag a comparison or `switch` only when the non-literal operand's type is the *wide* `string` — not when it's already a narrow string-literal union/enum being compared to one of its own members. That distinction matters because TypeScript's idiomatic "enum" (a `type Foo = 'a' | 'b'` union) is itself expressed via string literals, so a syntax-only rule can't tell a real violation from already-correct code.

* Exempt inside functions with a TS type-predicate return type (`function isFoo(x): x is Foo`) or a return type that is itself a narrow string-literal union — the direct equivalent of Rust's `from`/`from_str`/`try_from`/`deserialize` exemption.
* Also exempt for `typeof` checks and `KeyboardEvent.key` comparisons (a DOM API contract, like Rust's `tracing::field::Field::name()` case), and for comparisons against the empty string literal (presence checks, not domain state).
* A few remaining sites are legitimately string-based (env var/localStorage boolean flags, `window.location.protocol`, PrimeVue's own untyped prop contracts) and carry a `// eslint-disable-next-line local/no-string-literal-control-flow -- reason` comment.
* Backend response fields the Rust side serializes as plain strings (e.g. `status: string` on report/activity rows) are normalized once via shared helpers like `frontend/src/utils/backupStatus.ts` rather than compared ad hoc at each call site.
* Run it locally with `npm run lint` in `frontend/`; it runs alongside the existing ESLint rules, no separate command needed.
