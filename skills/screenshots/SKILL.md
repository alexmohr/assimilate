<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Screenshots Skill

Use when:

* a change alters what any page of the app looks like (a view, panel, dialog,
  table, chart, state message, colour, spacing, or wording on screen)
* adding or changing a documentation page that shows a UI feature
* changing the project website (`website/`) or anything it shows
* adding or changing demo data in `.devcontainer/demo/seed-demo.sh`

Read `skills/documentation/SKILL.md` for where screenshots go in the docs, and
`skills/ui-design/SKILL.md` for the UI rules themselves.

## Required

* **Every screenshot comes from the script.** All images under
  `docs/assets/screenshots/` and `website/assets/shots/` are produced by
  `frontend/e2e/docs.screenshots.ts`. Never commit a hand-made capture: a new
  image gets a step in that script, so the next full recapture reproduces it
  with the same viewport, theme, and data as every other image.
* **CI enforces freshness.** `npm run screenshots` records a fingerprint of
  everything that shapes the images in `frontend/e2e/screenshots.fingerprint`.
  That covers `frontend/src` (minus tests, test utilities, mocks and generated
  API types), `frontend/public`, `frontend/index.html`, the capture script, its
  config and fixtures, and `seed-demo.sh`. The **Screenshot freshness** CI job
  fails when the committed fingerprint no longer matches those files, so any PR
  touching them must commit fresh screenshots. Recapture the whole set in one
  run rather than single images, so every image reflects the same build and the
  same seed. Never edit or regenerate the fingerprint by hand: it is only
  written after a full successful capture.
* **Capture from the seeded demo, on current `main`.** Rebase onto `main`
  first, then capture from a freshly started demo. Images from an old branch
  or a hand-edited database show a UI that no longer exists.
* **New UI needs demo data first.** If the screenshot needs a scenario the
  demo doesn't have, add it to `seed-demo.sh` (see
  `skills/documentation/SKILL.md`), then add the capture step.
* **Look at the images before committing.** Check each changed image for
  spinners, half-loaded tables, empty states that should have data, and the
  wrong theme. The script waits for these, but a new step can still race.

## What the script produces

`npm run screenshots` (from `frontend/`) runs
`frontend/e2e/docs.screenshots.ts` with
`frontend/playwright.screenshots.config.ts`. The regular e2e run never picks it
up, because the default config matches `*.spec.ts` and this file is
`*.screenshots.ts`.

| Output | Where | Settings |
|---|---|---|
| Documentation screenshots | `docs/assets/screenshots/<name>.png` | Light theme, 1280×800 viewport at 2× density |
| Website restore crop | `website/assets/shots/restore-dark.png` | Dark theme, 1280×900 at 2× |
| Website dependency-skip run | `website/assets/shots/dependency-skip.png` | Light theme, 1280×800 at 2× |
| Website phone view | `website/assets/shots/phone-dashboard.png` | 390×844 at 3× |
| Website tablet view | `website/assets/shots/tablet-schedules.png` | 820×1180 at 2× |

The website markup states the phone and tablet image sizes (1170×2532 and
1640×2360). If you change those viewports, update the `width`/`height`
attributes in `website/index.html` too.

## Refreshing from CI (no Docker needed)

The **Playwright E2E** job captures fresh screenshots of the PR's commit from
its seeded demo, before the e2e tests run, and uploads them with the matching
fingerprint as the `screenshots` artifact. When **Screenshot freshness** fails:

1. Wait for that PR's Playwright E2E job to finish and download the
   `screenshots` artifact.
2. Unpack it at the repository root. It holds `docs/assets/screenshots/`,
   `website/assets/shots/` and `frontend/e2e/screenshots.fingerprint`.
3. Look at the changed images, then commit them together with the
   fingerprint.

A commit that changes the inputs again makes the artifact stale, so take the
artifact from the run of the commit you're about to build on.

## Capturing locally

1. Rebase onto `main`.
2. Start the demo. This always tears down the old containers and volumes:

   ```bash
   .devcontainer/start.sh --demo
   ```

3. Wait for the seed to finish (`==> Demo data seeded successfully.`). The
   seed restarts the server near the end, and the script itself waits for the
   agents to reconnect.
4. Recapture everything:

   ```bash
   cd frontend && npm run screenshots
   ```

   * `E2E_BASE_URL` points it at a demo that isn't on
     `http://localhost:8080`.
   * `PLAYWRIGHT_CHROMIUM_PATH` uses a preinstalled Chromium instead of
     running `playwright install`.

5. Review the changed images (`git status docs/assets/screenshots
   website/assets/shots`). For the website images, preview the site with
   `scripts/build-website.sh --serve`.
6. Run `pre-commit`. Its `reuse-annotate` hook adds the `.license` file next to
   any new PNG.
7. Commit the images together with `frontend/e2e/screenshots.fingerprint`,
   which the run updated. `python3 scripts/screenshot_fingerprint.py --check`
   confirms it matches.

## Adding a screenshot

* **Docs image:** add a step to the matching `test(...)` block in
  `docs.screenshots.ts`. Use `shot(page, '<name>')` for the full viewport, or
  `shotOf(locator, '<name>')` for one element. Name it in lowercase
  kebab-case after the page or view (`schedule-detail`, `repo-detail`).
* **Website image:** add it to the `website` describe block and write it to
  `WEBSITE_DIR` with `shotOfTo(...)` or `page.screenshot(...)`.
* **Wait for real state.** Find seeded entries by name with
  `seededIdByName(page, endpoint, name)` from `e2e/fixtures.ts`, never by
  assumed ids. Wait on what the page shows when it is done, for example
  `waitForIndex` for the archive browser, rather than on a fixed sleep or on a
  loading note going away. `shot`/`shotOf` already wait for `BaseSpinner` to
  clear.

## Checklist

* [ ] Branch rebased onto `main` before capturing
* [ ] Demo freshly started and fully seeded
* [ ] Full `npm run screenshots` run, or the CI `screenshots` artifact; every changed image looked at
* [ ] `frontend/e2e/screenshots.fingerprint` committed with the images (`python3 scripts/screenshot_fingerprint.py --check` passes)
* [ ] New images come from a script step, not a manual capture
* [ ] `.license` files present for new PNGs (pre-commit adds them)
* [ ] Docs pages and the website reference the right file names
