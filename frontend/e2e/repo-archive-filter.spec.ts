// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('archive filter via ?archive= query parameter', () => {
  test('AC-E1: navigating to repo detail with ?archive=<name> shows the filter banner', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    // Navigate to repos list and click server-daily to get its ID
    await page.goto('/repos')
    await page.getByText('server-daily').click()
    await page.waitForURL(/\/repos\/\d+/)
    const repoUrl = page.url()
    const repoId = new URL(repoUrl).pathname.match(/\/repos\/(\d+)/)?.[1]
    expect(repoId).toBeTruthy()

    // Go to the Archives tab to see archive names
    await page.getByRole('button', { name: 'Archives' }).click()
    await page.waitForURL(/tab=archives/)

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

    // Only one archive row should be visible
    await expect(page.locator('.archive-row')).toHaveCount(1)

    // The matching archive row should be selected
    await expect(page.locator('.archive-row.selected')).toBeVisible()
  })

  test('AC-E2: clicking "Show all archives" restores the full archive list', async ({ page }) => {
    await loginAsAdmin(page)

    // Navigate to repos list and click server-daily to get its ID
    await page.goto('/repos')
    await page.getByText('server-daily').click()
    await page.waitForURL(/\/repos\/\d+/)
    const repoUrl = page.url()
    const repoId = new URL(repoUrl).pathname.match(/\/repos\/(\d+)/)?.[1]
    expect(repoId).toBeTruthy()

    // Go to the Archives tab to see archive names and get the total count
    await page.getByRole('button', { name: 'Archives' }).click()
    await page.waitForURL(/tab=archives/)

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

    // Verify the banner is visible and only 1 archive is shown
    await expect(page.locator('.archive-filter-banner')).toBeVisible()
    await expect(page.locator('.archive-row')).toHaveCount(1)

    // Click "Show all archives"
    await page.getByRole('button', { name: 'Show all archives' }).click()

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
