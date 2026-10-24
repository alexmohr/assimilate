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
* **CI enforces freshness, one image at a time.** Every image has a record
  under `frontend/e2e/screenshot-fingerprints/` (mirroring the image's path,
  plus `.json`) naming the test that takes it, the page it shows, and a hash
  of its inputs:
  * the app shell, shared by every image: the import closure of
    `frontend/src/main.ts` (without the views the router loads lazily),
    `frontend/index.html`, `frontend/public`, the capture config, and
    `seed-demo.sh`
  * the import closure of the image's own page: its route's view, with every
    component, composable and asset it references
  * the capture script's shared code (helpers and hooks, outside any
    `test(...)` block), the image's own `test(...)` block, and the fixtures
    the script imports

  Tests, test utilities, mocks, generated API types, comments and formatting
  are left out. The **Screenshot freshness** CI job fails when a record no
  longer matches, so a PR must recapture exactly the images whose view (or
  capture step) it changed. A change to one view never touches another
  view's images or records, so two PRs on different views don't conflict.
  A change to the app shell or the seed still makes every image stale.
  Never write or edit a record by hand: `npm run screenshots` writes them
  after a full successful capture.
* **Capture everything, commit what changed.** `npm run screenshots` captures
  every image into `frontend/test-results/screenshots/`, then
  `scripts/screenshot_fingerprint.py --update` copies over only the images
  whose record changed and deletes the ones the script no longer takes. The
  other images keep their committed bytes even though a fresh capture of them
  differs (the demo's dates move with the clock).
* **Keep test titles unique.** A record finds its capture step by the
  `test(...)` title, so two tests with one title are an error, and renaming a
  test makes its images stale.
* **Capture from the seeded demo, on current `main`.** Rebase onto `main`
  first, then capture from a freshly started demo. Images from an old branch
  or a hand-edited database show a UI that no longer exists. On a rebase that
  conflicts in screenshots or records, take `main`'s side for both and
  recapture: only the images your change makes stale come back.
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
| Website dependency Used by | `website/assets/shots/dependency-used-by.png` | Light theme, 1280×800 at 2× |
| Website phone view | `website/assets/shots/phone-dashboard.png` | 390×844 at 3× |
| Website tablet view | `website/assets/shots/tablet-schedules.png` | 820×1180 at 2× |

The website markup states the phone and tablet image sizes (1170×2532 and
1640×2360). If you change those viewports, update the `width`/`height`
attributes in `website/index.html` too.

## Refreshing from CI (no Docker needed)

The **Screenshot capture** job starts its own seeded demo, captures every
image of the PR's commit, keeps only those whose inputs changed, and uploads
them with their records as the `screenshots` artifact. It fails while there
are any, and its job summary lists images the script no longer takes. It
loads the same demo image the Playwright E2E job uses (built once by the
**Demo image** job) but runs its own containers, so neither changes the data
the other sees. When **Screenshot freshness** or **Screenshot capture**
fails:

1. Wait for that PR's Screenshot capture job to finish and download the
   `screenshots` artifact.
2. Unpack it at the repository root. It holds only the changed images under
   `docs/assets/screenshots/` and `website/assets/shots/`, and their records
   under `frontend/e2e/screenshot-fingerprints/`.
3. Delete any image the job summary lists as no longer captured, with its
   `.license` file and record.
4. Look at the changed images, then commit them together with their records.

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
   website/assets/shots`). Only images whose inputs changed are rewritten.
   For the website images, preview the site with
   `scripts/build-website.sh --serve`.
6. Run `pre-commit`. Its `reuse-annotate` hook adds the `.license` file next to
   any new PNG.
7. Commit the images together with their records under
   `frontend/e2e/screenshot-fingerprints/`, which the run updated.
   `python3 scripts/screenshot_fingerprint.py --check` confirms they match.

## Adding a screenshot

* **Docs image:** add a step to the matching `test(...)` block in
  `docs.screenshots.ts`. Use `shot(page, '<name>')` for the full viewport, or
  `shotOf(locator, '<name>')` for one element. Name it in lowercase
  kebab-case after the page or view (`schedule-detail`, `repo-detail`).
* **Website image:** add it to the `website` describe block and write it to
  `WEBSITE_DIR` with `shotTo(...)` or `shotOfTo(...)`.
* **Always go through these helpers**, never a bare `page.screenshot(...)`:
  they write the image to the capture directory together with the test and
  URL its record is built from. An image outside them is not recorded, so
  the next `npm run screenshots` deletes it.
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
* [ ] Records under `frontend/e2e/screenshot-fingerprints/` committed with the images (`python3 scripts/screenshot_fingerprint.py --check` passes)
* [ ] New images come from a script step, not a manual capture
* [ ] `.license` files present for new PNGs (pre-commit adds them)
* [ ] Docs pages and the website reference the right file names
