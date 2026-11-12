// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expandAllArchiveGroups, expect, loginAsAdmin, test } from './fixtures'
import type { Locator, Page } from '@playwright/test'

/** The archive list's rows; each is a button named "Open archive <name>". */
function archiveRows(page: Page): Locator {
  return page.locator('.archives-panel').getByTestId('archive-row')
}

/** A folder of the open archive: the button its row's name is. */
function folderEntry(browserPanel: Locator, name: string | RegExp): Locator {
  return browserPanel
    .getByTestId('browser-entry')
    .getByRole('button', { name, exact: typeof name === 'string' })
}

// media-weekly has 12 weeks of seeded archives and isn't asserted on by name
// or exact count anywhere else in the suite, unlike server-daily's
// web-server-01 archives - the safest repo to actually delete an archive
// from in an e2e run.
async function navigateToMediaWeeklyArchives(page: Page): Promise<void> {
  await page.goto('/repos', { waitUntil: 'commit' })
  await page.getByText('media-weekly', { exact: true }).click()
  await page.waitForURL(/\/repos\/\d+/, { waitUntil: 'commit' })
  await page.getByRole('tab', { name: 'Archives', exact: true }).click()
  await page.waitForLoadState('networkidle')
}

test.describe('Archive browsing & diff journey', () => {
  test('archives tab loads showing archive entries with names, dates, and hosts', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('tab', { name: 'Archives' })).toBeVisible()
    await expect(page.locator('.panel-title').filter({ hasText: 'Archives' })).toBeVisible()
    await expandAllArchiveGroups(page)

    const firstRow = archiveRows(page).first()
    await expect(firstRow).toBeVisible({ timeout: 30_000 })
    await expect(firstRow.locator('.browser-row-title')).not.toBeEmpty()
    await expect(firstRow.locator('time.browser-row-time')).toBeVisible()
  })

  test('archive list contains web-server-01 backup entries', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await expect(page.getByText(/web-server-01-backup/).first()).toBeVisible()
  })

  test('clicking an archive shows file tree browser', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()

    const browserPanel = page.locator('.browser-panel').last()
    await expect(browserPanel).toBeVisible()
    await expect(browserPanel.getByRole('tab', { name: 'Files' })).toHaveAttribute(
      'aria-selected',
      'true',
    )
    await expect(browserPanel.locator('.path-crumbs')).toBeVisible()
  })

  test('file browser shows directory entries with names and modified dates', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()

    // Indexing an archive's contents on first access can take a while - allow
    // more time than the default 5s before the directory listing renders.
    // The demo backs up a mktemp -d directory, so the archive root contains a
    // single "tmp" entry rather than the backed-up paths (etc/, var/) directly.
    const browserPanel = page.locator('.browser-panel').last()
    const tmpEntry = folderEntry(browserPanel, 'tmp')
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    // A folder's row says it is one (or how many files it holds).
    await expect(browserPanel.getByTestId('folder-files').first()).toBeVisible()

    // The backed-up directory holds restore-example.txt, a file whose row
    // carries its modified date.
    await tmpEntry.click()
    await folderEntry(browserPanel, /^tmp\./).click()
    const file = browserPanel
      .getByTestId('browser-entry')
      .filter({ hasText: 'restore-example.txt' })
    await expect(file).toBeVisible({ timeout: 30_000 })
    await expect(file).toContainText(/modified /)
  })

  test('file browser breadcrumb shows root path', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()

    // The root crumb is the archive's "/".
    const crumbs = page.locator('.browser-panel').last().locator('.path-crumbs .crumb')
    await expect(crumbs).toHaveCount(1, { timeout: 30_000 })
    await expect(crumbs.first()).toHaveClass(/crumb-root/)
    await expect(crumbs.first()).toHaveText('/')
  })

  test('clicking a directory in file browser navigates into it', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()

    // The demo backs up a mktemp -d directory, so the archive root's sole entry is "tmp".
    const browserPanel = page.locator('.browser-panel').last()
    const tmpEntry = folderEntry(browserPanel, 'tmp')
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    await tmpEntry.click()

    await expect(browserPanel.locator('.path-crumbs')).toContainText('tmp')
  })

  // The standalone /archives page reaches the same browser through
  // ArchiveFileBrowser; it used to carry its own copy of the browser's markup
  // and state machine, with no coverage of either.
  test('standalone archives page browses the selected archive', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/archives')
    await page.waitForURL('/archives')

    const repoSelect = page.locator('.repo-selector select')
    await expect(repoSelect).toBeVisible({ timeout: 15_000 })

    await repoSelect.selectOption({ index: 1 })

    // The two-pane layout mounts with the repository, so the placeholder is
    // the state between picking a repository and picking an archive.
    const browserPanel = page.locator('.browser-panel')
    await expect(browserPanel).toContainText('Pick an archive')

    await expandAllArchiveGroups(page)
    const archiveRow = archiveRows(page).first()
    await expect(archiveRow).toBeVisible({ timeout: 15_000 })
    await archiveRow.click()

    // Indexing an archive's contents on first access can take a while.
    await expect(browserPanel.getByRole('tab', { name: 'Files' })).toHaveAttribute(
      'aria-selected',
      'true',
      { timeout: 30_000 },
    )
    await expect(browserPanel.locator('.path-crumbs .crumb-root')).toBeVisible({
      timeout: 30_000,
    })
    await expect(browserPanel.getByTestId('browser-entry').first()).toBeVisible({
      timeout: 30_000,
    })
  })

  test('standalone archives page navigates into a directory', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/archives')
    await page.waitForURL('/archives')

    const repoSelect = page.locator('.repo-selector select')
    await expect(repoSelect).toBeVisible({ timeout: 15_000 })
    await repoSelect.selectOption({ index: 1 })

    await expandAllArchiveGroups(page)
    const archiveRow = archiveRows(page).first()
    await expect(archiveRow).toBeVisible({ timeout: 15_000 })
    await archiveRow.click()

    // The demo backs up a mktemp -d directory, so the archive root's sole
    // entry is "tmp".
    const browserPanel = page.locator('.browser-panel')
    const tmpEntry = folderEntry(browserPanel, 'tmp')
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    await tmpEntry.click()

    await expect(browserPanel.locator('.path-crumbs')).toContainText('tmp')
  })

  // Issue #198: every directory opened is a browser history entry, so Back
  // walks back up the directories visited until the archive root, then closes
  // the archive, and only then leaves the page.
  test('browser Back walks up the visited directories, then leaves the page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForURL('/repos')
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()

    const browserPanel = page.locator('.browser-panel').last()
    const crumbs = browserPanel.locator('.path-crumbs .crumb')
    const pathParam = (): string | null => new URL(page.url()).searchParams.get('path')
    const browseParam = (): string | null => new URL(page.url()).searchParams.get('browse')

    // The demo backs up a mktemp -d directory: the archive root holds "tmp",
    // which holds the single tmp.XXXXXXXXXX directory that was backed up.
    const tmpEntry = folderEntry(browserPanel, 'tmp')
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    await expect(crumbs).toHaveCount(1)
    expect(pathParam()).toBeNull()
    // The open archive is in the URL too.
    await expect.poll(browseParam).toMatch(/web-server-01-backup/)

    await tmpEntry.click()
    await expect(crumbs).toHaveCount(2)
    await expect.poll(pathParam).toBe('/tmp')

    const backedUpDir = folderEntry(browserPanel, /^tmp\./)
    await expect(backedUpDir).toBeVisible({ timeout: 30_000 })
    const backedUpName = (await backedUpDir.innerText()).trim()
    await backedUpDir.click()
    await expect(crumbs).toHaveCount(3)
    await expect(crumbs.last()).toHaveText(backedUpName)
    await expect.poll(pathParam).toBe(`/tmp/${backedUpName}`)
    // The rest of the query survives the directory being added to it.
    expect(new URL(page.url()).searchParams.get('tab')).toBe('archives')

    await page.goBack()
    await expect(crumbs).toHaveCount(2)
    await expect(crumbs.last()).toHaveText('tmp')
    await expect(backedUpDir).toBeVisible({ timeout: 30_000 })
    await expect.poll(pathParam).toBe('/tmp')

    await page.goBack()
    await expect(crumbs).toHaveCount(1)
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    await expect.poll(pathParam).toBeNull()
    await expect(page).toHaveURL(/\/repos\/1\?tab=archives/)

    // From the archive root, Back closes the archive...
    await page.goBack()
    await expect(browserPanel.getByText('Pick an archive')).toBeVisible()
    await expect.poll(browseParam).toBeNull()

    // ...and then it is ordinary navigation again.
    await page.goBack()
    await expect(page).toHaveURL(/\/repos$/)
  })

  test('a link names the archive and the directory to open in it', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)
    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()
    await expect.poll(() => new URL(page.url()).searchParams.get('browse')).not.toBeNull()
    const archiveName = new URL(page.url()).searchParams.get('browse')!

    // A fresh load of the link, as from a bookmark or a shared URL.
    await page.goto(`/repos/1?tab=archives&browse=${encodeURIComponent(archiveName)}&path=%2Ftmp`)
    const browserPanel = page.locator('.browser-panel').last()
    const crumbs = browserPanel.locator('.path-crumbs .crumb')
    await expect(crumbs).toHaveCount(2, { timeout: 30_000 })
    await expect(crumbs.last()).toHaveText('tmp')
    await expect(browserPanel).toContainText(archiveName)
  })

  test('the open archive offers download, restore and delete in its menu', async ({ page }) => {
    // These used to hang off the file table's "." row, with delete rendered at
    // zero opacity until the pointer happened to hover that row; the archive
    // card now keeps everything that acts on the whole archive in one menu.
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()

    const browserPanel = page.locator('.browser-panel').last()
    const menuButton = browserPanel.getByRole('button', { name: 'Archive actions' })
    await expect(menuButton).toBeVisible({ timeout: 30_000 })
    await menuButton.click()
    const menu = browserPanel.getByRole('menu')
    await expect(menu.getByRole('menuitem', { name: 'Download' })).toBeVisible()
    await expect(menu.getByRole('menuitem', { name: 'Restore whole archive...' })).toBeVisible()
    await expect(menu.getByRole('menuitem', { name: 'Delete archive...' })).toBeVisible()

    // The archive says what it is beside its files: its name, and a facts
    // line ending in what it added to the repository.
    await expect(browserPanel.locator('.archive-head-name')).toContainText('web-server-01-backup')
    await expect(browserPanel.locator('.archive-head-facts')).toContainText(/\+.* new/)
  })

  test('archive tags API endpoint returns the seeded tags, not just an array', async ({ page }) => {
    await loginAsAdmin(page)
    const archivesRes = await page.request.get('/api/repos/1/archives')
    expect(archivesRes.ok()).toBeTruthy()

    const archives: { name: string; start: string }[] = await archivesRes.json()
    expect(archives.length).toBeGreaterThan(0)

    // The seed tags web-server-01's newest and third-newest archives, so look
    // there rather than at whichever archive happens to sort first: other
    // hosts back up to this repository too. Asking an untagged archive returns
    // `[]`, which the old `Array.isArray` assertion accepted - so this test
    // passed while covering none of the tag response it is named for.
    const newest = archives
      .filter((a) => a.name.startsWith('web-server-01-backup-'))
      .sort((a, b) => b.start.localeCompare(a.start))
      .slice(0, 4)
    expect(newest.length).toBeGreaterThan(0)

    const responses = []
    for (const archive of newest) {
      const res = await page.request.get(
        `/api/repos/1/archives/${encodeURIComponent(archive.name)}/tags`,
      )
      expect(res.ok()).toBeTruthy()
      const tags = (await res.json()) as { tag: string; archive_name: string; repo_id: number }[]
      expect(Array.isArray(tags)).toBeTruthy()
      if (tags.length > 0) responses.push({ archive, tags })
    }

    expect(responses.length).toBeGreaterThan(0)
    const [{ archive, tags }] = responses
    expect(tags[0].archive_name).toBe(archive.name)
    expect(tags[0].repo_id).toBe(1)
    expect(typeof tags[0].tag).toBe('string')
    expect(tags[0].tag.length).toBeGreaterThan(0)
  })

  test('archive diff API returns structured results for two archives', async ({ page }) => {
    await loginAsAdmin(page)
    const archivesRes = await page.request.get('/api/repos/1/archives')
    expect(archivesRes.ok()).toBeTruthy()

    const archives: { name: string }[] = await archivesRes.json()
    expect(archives.length).toBeGreaterThanOrEqual(2)

    const [first, second] = archives
    const diffRes = await page.request.get(
      `/api/repos/1/archives/diff?archive1=${encodeURIComponent(first.name)}&archive2=${encodeURIComponent(second.name)}`,
    )
    expect(diffRes.ok()).toBeTruthy()

    const diff: unknown = await diffRes.json()
    expect(diff).toBeDefined()
  })

  test('archives tab is accessible from repository detail overview tab', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1')
    await page.waitForLoadState('networkidle')

    const archivesTab = page.getByRole('tab', { name: 'Archives' })
    await expect(archivesTab).toBeVisible()

    await archivesTab.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/tab=archives/)
    await expect(page.locator('.panel-title').filter({ hasText: 'Archives' })).toBeVisible()
  })

  test('deleting an archive shows an in-progress state and the archive disappears once done', async ({
    page,
  }) => {
    // borg delete + the automatic compact that follows it can take a while
    // even on the demo's small repos, so give it plenty of room (matching
    // import.spec.ts's allowance for the same class of slow borg operation).
    test.setTimeout(180_000)

    await loginAsAdmin(page)
    await navigateToMediaWeeklyArchives(page)
    await expandAllArchiveGroups(page)

    const firstRow = archiveRows(page).first()
    await expect(firstRow).toBeVisible({ timeout: 30_000 })
    const archiveName = (await firstRow.locator('.browser-row-title').innerText()).trim()

    // Deleting is in the open archive's menu, not on the row.
    await firstRow.click()
    const browserPanel = page.locator('.browser-panel').last()
    await browserPanel.getByRole('button', { name: 'Archive actions' }).click()
    await browserPanel.getByRole('menuitem', { name: 'Delete archive...' }).click()

    // The confirm button is reached through the dialog's footer.
    await page
      .locator('.modal-footer')
      .getByRole('button', { name: 'Delete archive', exact: true })
      .click()

    // RepoDetailView marks the row as deleting synchronously, before the
    // DELETE request even goes out. Two earlier real races have since been
    // fixed at the app level rather than papered over here: RepoOpChanged's
    // stale-marker cleanup used to clear deletingArchiveNames immediately
    // against possibly-stale local state, racing DataChanged's own
    // list-refresh-driven prune for a delete that had just succeeded (it
    // now refetches before clearing); and the client used to learn a delete
    // had finished only via the generic DataChanged signal, forcing a full
    // list refetch-and-diff to notice this specific archive was gone. The
    // server now also broadcasts a precise ArchiveDeleted { archive_name }
    // event the moment this archive's delete + auto-compact finish, which
    // RepoDetailView applies directly and synchronously - no refetch in the
    // loop for the common success path at all.
    //
    // What's left is a plain Playwright coordination gap, not an app race:
    // two separate `expect().toBeVisible()`/`expect().toBeDisabled()` calls
    // each poll the DOM on their own schedule, so the row can appear (proof
    // the marking above ran) and then disappear (the delete finishing) in
    // the gap between the two polls, on a backend fast enough to complete
    // the whole delete+compact+notify cycle inside it. `toPass` re-reads
    // both properties together in one atomic check, so either the row is
    // caught in the pending state with both properties true at once, or the
    // retry loop keeps trying - it does not tolerate the row never
    // appearing at all, or being visible-but-not-disabled. A fixed, tight
    // interval (rather than toPass's default 100/250/500/1000ms backoff,
    // which samples far less often over 5s) matters here: a CI run once
    // caught the row visible-then-vanished with the default backoff still
    // in place, but the very next run never sampled it as visible at all -
    // the pending window on this backend can be narrower than what a
    // backing-off poll reliably lands inside.
    const pendingRow = archiveRows(page)
      .filter({ hasText: archiveName })
      .filter({ has: page.locator('.archive-row-deleting') })
    await expect(async () => {
      expect(await pendingRow.isVisible()).toBe(true)
      expect(await pendingRow.textContent()).toContain('Deleting')
    }).toPass({ timeout: 5_000, intervals: [20] })

    // While the delete (and the compact that automatically follows it) is
    // still running, the Overview tab's "Current operation" field should
    // reflect one of the two phases. Best-effort: on a fast demo repo this
    // window can be too short to reliably observe, so don't fail the test
    // over it - the definitive proof the whole pipeline ran is the archive
    // disappearing below.
    await page.getByRole('tab', { name: 'Overview', exact: true }).click()
    await page
      .getByText(/Deleting archive|Compacting repository/)
      .first()
      .waitFor({ state: 'visible', timeout: 5_000 })
      .catch(() => {})

    await page.getByRole('tab', { name: 'Archives', exact: true }).click()
    await expandAllArchiveGroups(page)
    await expect(archiveRows(page).filter({ hasText: archiveName })).toHaveCount(0, {
      timeout: 60_000,
    })
  })
})
