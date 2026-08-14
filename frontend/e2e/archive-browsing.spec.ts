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
  await page.getByRole('button', { name: 'Archives', exact: true }).click()
  await page.waitForLoadState('networkidle')
}

test.describe('Archive browsing & diff journey', () => {
  test('archives tab loads showing archive entries with names, dates, and hosts', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('button', { name: 'Archives' })).toBeVisible()
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
    await expect(page.locator('.archive-breadcrumb')).toBeVisible()

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

    await expect(page.locator('.archive-breadcrumb').getByText('~')).toBeVisible()
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

    await expect(page.locator('.archive-breadcrumb')).toContainText('tmp')
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

    const archivesTab = page.getByRole('button', { name: 'Archives' })
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

    // RepoDetailView marks the row as deleting synchronously, before the
    // DELETE request even goes out - but the DataChanged notification that
    // follows the server-side delete+compact triggers a refetch of the
    // archive list, which prunes this row (button included) from the DOM.
    // On a fast backend that whole round trip can complete faster than a
    // single assertion-polling tick, so CI has intermittently - and on one
    // run, on every retry - missed the pending state entirely (see PR #408
    // discussion). Delaying just that one refetch guarantees the pending
    // state a real window to be observed without weakening what's actually
    // asserted below: the button must still become visible and disabled,
    // unconditionally.
    let delayedRefetch = false
    await page.route('**/api/repos/*/archives', async (route) => {
      if (route.request().method() === 'GET' && !delayedRefetch) {
        delayedRefetch = true
        await new Promise((resolve) => setTimeout(resolve, 2_000))
      }
      await route.continue()
    })

    await page.getByRole('button', { name: 'Delete Archive', exact: true }).click()

    const pendingBtn = page
      .locator('.archive-row', { hasText: archiveName })
      .locator('button[title="Deletion in progress"]')
    await expect(pendingBtn).toBeVisible({ timeout: 5_000 })
    await expect(pendingBtn).toBeDisabled()

    await page.unroute('**/api/repos/*/archives')

    // While the delete (and the compact that automatically follows it) is
    // still running, the Overview tab's "Current Operation" field should
    // reflect one of the two phases. Best-effort: on a fast demo repo this
    // window can be too short to reliably observe, so don't fail the test
    // over it - the definitive proof the whole pipeline ran is the archive
    // disappearing below.
    await page.getByRole('button', { name: 'Overview', exact: true }).click()
    await page
      .getByText(/Deleting archive|Compacting repository/)
      .first()
      .waitFor({ state: 'visible', timeout: 5_000 })
      .catch(() => {})

    await page.getByRole('button', { name: 'Archives', exact: true }).click()
    await expandAllArchiveGroups(page)
    await expect(page.locator('.archive-name', { hasText: archiveName })).not.toBeVisible({
      timeout: 60_000,
    })
  })
})
