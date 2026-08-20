// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expandAllArchiveGroups, expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

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

    const firstRow = page.locator('.archive-row').first()
    await expect(firstRow).toBeVisible({ timeout: 30_000 })
    await expect(firstRow.locator('.archive-name')).toBeVisible()
    await expect(firstRow.locator('.archive-date')).toBeVisible()
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
    await page.waitForTimeout(1000)

    await expect(page.locator('.browser-title').filter({ hasText: /Files/ })).toBeVisible()
    await expect(page.locator('.path-crumbs')).toBeVisible()

    const browserPanel = page.locator('.browser-panel').last()
    await expect(browserPanel).toBeVisible()
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
    await page.waitForTimeout(1000)

    // Indexing an archive's contents on first access can take a while - allow
    // more time than the default 5s before the directory listing renders.
    // The demo backs up a mktemp -d directory, so the archive root contains a
    // single "tmp" entry rather than the backed-up paths (etc/, var/) directly.
    const browserPanel = page.locator('.browser-panel').last()
    await expect(browserPanel.getByText('Name')).toBeVisible({ timeout: 30_000 })
    await expect(browserPanel.getByText('Modified')).toBeVisible()
    await expect(browserPanel.getByText('tmp', { exact: true })).toBeVisible()
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
    await page.waitForTimeout(1000)

    await expect(page.locator('.path-crumbs').getByText('~')).toBeVisible()
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
    await page.waitForTimeout(1000)

    // The demo backs up a mktemp -d directory, so the archive root's sole entry is "tmp".
    const browserPanel = page.locator('.browser-panel').last()
    const tmpEntry = browserPanel.getByText('tmp', { exact: true })
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    await tmpEntry.click()
    await page.waitForTimeout(1000)

    await expect(page.locator('.path-crumbs')).toContainText('tmp')
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
    await expect(browserPanel).toContainText('Select an archive to browse its contents.')

    const archiveRow = page.locator('.archives-panel .td-mono').first()
    await expect(archiveRow).toBeVisible({ timeout: 15_000 })
    await archiveRow.click()

    // Indexing an archive's contents on first access can take a while.
    await expect(browserPanel.locator('.browser-title')).toContainText('Files', {
      timeout: 30_000,
    })
    await expect(browserPanel.locator('.path-crumbs').getByText('~')).toBeVisible({
      timeout: 30_000,
    })
    await expect(browserPanel.getByText('Name')).toBeVisible({ timeout: 30_000 })
  })

  test('standalone archives page navigates into a directory', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/archives')
    await page.waitForURL('/archives')

    const repoSelect = page.locator('.repo-selector select')
    await expect(repoSelect).toBeVisible({ timeout: 15_000 })
    await repoSelect.selectOption({ index: 1 })

    const archiveRow = page.locator('.archives-panel .td-mono').first()
    await expect(archiveRow).toBeVisible({ timeout: 15_000 })
    await archiveRow.click()

    // The demo backs up a mktemp -d directory, so the archive root's sole
    // entry is "tmp".
    const browserPanel = page.locator('.browser-panel')
    const tmpEntry = browserPanel.getByText('tmp', { exact: true })
    await expect(tmpEntry).toBeVisible({ timeout: 30_000 })
    await tmpEntry.click()

    await expect(browserPanel.locator('.path-crumbs')).toContainText('tmp')
  })

  test('archive tags API endpoint is accessible and returns structured data', async ({ page }) => {
    await loginAsAdmin(page)
    const archivesRes = await page.request.get('/api/repos/1/archives')
    expect(archivesRes.ok()).toBeTruthy()

    const archives: { name: string }[] = await archivesRes.json()
    expect(archives.length).toBeGreaterThan(0)

    const tagsRes = await page.request.get(
      `/api/repos/1/archives/${encodeURIComponent(archives[0].name)}/tags`,
    )
    expect(tagsRes.ok()).toBeTruthy()
    const tags: unknown = await tagsRes.json()
    expect(Array.isArray(tags)).toBeTruthy()
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

    const firstRow = page.locator('.archive-row').first()
    await expect(firstRow).toBeVisible({ timeout: 30_000 })
    const archiveName = await firstRow.locator('.archive-name').innerText()

    const deleteBtn = firstRow.locator('button[title="Delete archive"]')
    await expect(deleteBtn).toBeVisible()
    await deleteBtn.click()

    // The row buttons carry the same "Delete archive" title, so the confirm
    // button is reached through the dialog rather than by name alone.
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
    const pendingBtn = page
      .locator('.archive-row', { hasText: archiveName })
      .locator('button[title="Deletion in progress"]')
    await expect(async () => {
      expect(await pendingBtn.isVisible()).toBe(true)
      expect(await pendingBtn.isDisabled()).toBe(true)
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
    await expect(page.locator('.archive-name', { hasText: archiveName })).not.toBeVisible({
      timeout: 60_000,
    })
  })
})
