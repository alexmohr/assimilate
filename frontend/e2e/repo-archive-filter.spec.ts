// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expandAllArchiveGroups, expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

/** Navigate to server-daily's Archives tab (all host groups expanded) and return its repo ID. */
async function goToServerDailyArchives(page: Page): Promise<string> {
  await page.goto('/repos')
  await page.getByText('server-daily').click()
  await page.waitForURL(/\/repos\/\d+/)
  const repoUrl = page.url()
  const repoId = new URL(repoUrl).pathname.match(/\/repos\/(\d+)/)?.[1]
  expect(repoId).toBeTruthy()

  await page.getByRole('button', { name: 'Archives' }).click()
  await page.waitForURL(/tab=archives/)
  await expandAllArchiveGroups(page)

  return repoId ?? ''
}

test.describe('archive filter via ?archive= query parameter', () => {
  test('AC-E1: navigating to repo detail with ?archive=<name> shows the filter banner', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    const repoId = await goToServerDailyArchives(page)

    // Read the first archive name from the list
    const firstArchiveName = page.locator('.archive-row .archive-name').first()
    await expect(firstArchiveName).toBeVisible()
    const archiveName = (await firstArchiveName.textContent()) ?? ''
    expect(archiveName).toBeTruthy()

    // Navigate to the same repo with archive filter query param
    await page.goto(`/repos/${repoId}?tab=archives&archive=${encodeURIComponent(archiveName)}`)
    await page.waitForURL(/tab=archives/)

    // Wait for the filter banner to appear
    const banner = page.locator('.archive-filter-banner')
    await expect(banner).toBeVisible()
    await expect(banner).toContainText(archiveName)

    // The archive browser and its controls are hidden while filtered to a
    // single archive - only the banner and the matching archive's file
    // browser are shown.
    await expect(page.locator('.archive-row')).toHaveCount(0)
    await expect(page.locator('.archive-controls')).not.toBeVisible()
    await expect(page.locator('.browser-title')).toContainText(archiveName)
  })

  test('AC-E2: clicking "Show all archives" restores the full archive list', async ({ page }) => {
    await loginAsAdmin(page)

    const repoId = await goToServerDailyArchives(page)

    // Get the first archive name
    const firstArchiveName = page.locator('.archive-row .archive-name').first()
    await expect(firstArchiveName).toBeVisible()
    const archiveName = (await firstArchiveName.textContent()) ?? ''
    expect(archiveName).toBeTruthy()

    // Get the total number of archive rows (should be >1 for server-daily)
    const totalArchiveRows = page.locator('.archive-row')
    const totalCount = await totalArchiveRows.count()
    expect(totalCount).toBeGreaterThan(1)

    // Navigate to the same repo with archive filter query param
    await page.goto(`/repos/${repoId}?tab=archives&archive=${encodeURIComponent(archiveName)}`)
    await page.waitForURL(/tab=archives/)

    // Verify the banner is visible and the archive browser is hidden
    await expect(page.locator('.archive-filter-banner')).toBeVisible()
    await expect(page.locator('.archive-row')).toHaveCount(0)

    // Click "Show all archives"
    await page.getByRole('button', { name: 'Show all archives' }).click()
    await expandAllArchiveGroups(page)

    // Wait for the filter banner to disappear
    await expect(page.locator('.archive-filter-banner')).not.toBeVisible()

    // Multiple archive rows should be visible again
    const restoredCount = await page.locator('.archive-row').count()
    expect(restoredCount).toBeGreaterThan(1)

    // URL should no longer contain ?archive=
    const currentUrl = new URL(page.url())
    expect(currentUrl.searchParams.has('archive')).toBe(false)
  })
})
