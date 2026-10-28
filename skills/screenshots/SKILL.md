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
* **Never commit the images yourself.** Two pull requests that both
  recapture an image conflict on a PNG, and a stale image costs a full
  re-run. So the **Screenshot capture** job commits them on `main` after the
  merge: it captures every image from a freshly seeded demo and commits the
  ones whose inputs changed (as `alexmohr-agent`), then redeploys the docs.
  A pull request must leave `docs/assets/screenshots/`,
  `website/assets/shots/` and `frontend/e2e/screenshot-fingerprints/` alone;
  the **Screenshot coverage** job fails it otherwise.
* **Every view needs a capture step.** The same job fails when a route of
  `frontend/src/router/index.ts` is visited by no step of
  `frontend/e2e/docs.screenshots.ts` (a `visit(page, '<path>')`, a `path:`
  property, or a `['<path>', '<name>']` pair; ids and other `${...}` parts
  stand for any value). A new view therefore ships with the code that takes
  its image, never with the image. The step must work: the **Screenshot
  capture** job runs the whole script on the pull request, and fails when a
  step does.
* **Which image is stale.** Every image has a record under
  `frontend/e2e/screenshot-fingerprints/` (mirroring the image's path, plus
  `.json`) naming the test that takes it, the page it shows, and a hash of its
  inputs, so `main` rewrites only the images whose inputs changed:
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
  are left out. Never write or edit a record by hand: `npm run screenshots`
  writes them after a full successful capture.
* **Docs may show an image before it exists.** Reference a new image from the
  docs or the website as usual (`![Alt](assets/screenshots/<name>.png)`).
  `scripts/build-website.sh` writes an untracked one-pixel stand-in for each
  referenced image that is not there yet, so the strict docs build passes
  until `main` has captured it; a name that no step of the capture script
  spells is still an error. Never commit a stand-in.
* **Capture everything, commit what changed.** `npm run screenshots` captures
  every image into `frontend/test-results/screenshots/`, then
  `scripts/screenshot_fingerprint.py --update` copies over only the images
  whose record changed (with their REUSE `.license` sidecars) and deletes the
  ones the script no longer takes. The other images keep their committed bytes
  even though a fresh capture of them differs (the demo's dates move with the
  clock).
* **Keep test titles unique.** A record finds its capture step by the
  `test(...)` title, so two tests with one title are an error, and renaming a
  test makes its images stale.
* **Capture from the seeded demo, on current `main`.** The job on `main` does
  this for you. To look at an image before the merge, rebase onto `main` first
  and capture from a freshly started demo (see "Looking at an image before the
  merge"); don't commit what you captured.
* **New UI needs demo data first.** If the screenshot needs a scenario the
  demo doesn't have, add it to `seed-demo.sh` (see
  `skills/documentation/SKILL.md`), then add the capture step.
* **Look at the images before merging.** Check each changed image (from the
  pull request's `screenshots` artifact, or a local capture) for spinners,
  half-loaded tables, empty states that should have data, and the wrong theme.
  The script waits for these, but a new step can still race.

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

## Looking at an image before the merge

The **Screenshot capture** job of a pull request captures every image from its
own seeded demo and uploads the ones whose inputs changed, with their records,
as the `screenshots` artifact. Download it to review them; nothing in it goes
into the pull request. When a step fails, the job fails with the Playwright
output.

To iterate faster, capture locally. This needs a working Docker daemon
(`docker info`).

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

5. Look at the changed images (`git status docs/assets/screenshots
   website/assets/shots`). For the website images, preview the site with
   `scripts/build-website.sh --serve`.
6. Throw them away before you commit (`git restore docs/assets/screenshots
   website/assets/shots frontend/e2e/screenshot-fingerprints` and
   `git clean -fd` on the same paths): the job on `main` commits them.

## What the job on main needs

The **Screenshot capture** job pushes to `main`. If `main` is protected, store a
token that may push to it as the repository secret `SCREENSHOT_PUSH_TOKEN`;
without it the job pushes with the workflow's own token. The token reaches only
the final commit step, which runs on `main` alone: the checkout keeps no
credentials, because the install and capture steps run a pull request's own
code. The commit says `[skip ci]` and the job dispatches `pages.yml` itself, so
the docs deploy once with the new images whichever token pushed. When `main`
moved on while the job ran, it does not push: the run for the newer commit does.

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

* [ ] Every new view has a capture step in `frontend/e2e/docs.screenshots.ts` (`python3 scripts/screenshot_fingerprint.py --coverage` passes)
* [ ] No file under `docs/assets/screenshots/`, `website/assets/shots/` or `frontend/e2e/screenshot-fingerprints/` is part of the pull request
* [ ] New images come from a script step, not a manual capture
* [ ] The pull request's **Screenshot capture** job passed, and the changed images in its `screenshots` artifact were looked at
* [ ] Docs pages and the website reference the right file names
