// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Archive browsing & diff journey', () => {
  test('archives tab loads showing archive entries with names, dates, and hosts', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('button', { name: 'Archives' })).toBeVisible()
    await expect(page.locator('.panel-title').filter({ hasText: 'Archives' })).toBeVisible()

    const firstRow = page.locator('.archive-row').first()
    await expect(firstRow).toBeVisible({ timeout: 30_000 })
    await expect(firstRow.locator('.archive-name')).toBeVisible()
    await expect(firstRow.locator('.archive-date')).toBeVisible()
  })

  test('archive list contains web-server-01 backup entries', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')

    await expect(page.getByText(/web-server-01-backup/).first()).toBeVisible()
  })

  test('clicking an archive shows file tree browser', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')

    await page
      .getByText(/web-server-01-backup/)
      .first()
      .click()
    await page.waitForTimeout(1000)

    await expect(page.locator('.panel-title').filter({ hasText: /Files/ })).toBeVisible()
    await expect(page.locator('.archive-breadcrumb')).toBeVisible()

    const browserPanel = page.locator('.browser-panel').last()
    await expect(browserPanel).toBeVisible()
  })

  test('file browser shows directory entries with names and modified dates', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')

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
})
