// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, seededGrowthArchive, test } from './fixtures'

// One of web-server-01's server-daily archives holds a release tarball
// and a debug log no other archive has (see start-agent.sh), and the seed
// works out its breakdown up front.
test.describe('Archive growth', () => {
  test('shows where an archive grew, folder by folder and by file', async ({ page }) => {
    await loginAsAdmin(page)
    const { repoId, name } = await seededGrowthArchive(page)

    await page.goto(`/repos/${repoId}?tab=archives`)
    await page.getByText('Flat', { exact: true }).first().click()
    await page.getByPlaceholder('Search name, host or repository').fill(name)
    await page.locator('.archive-row-body:visible').first().click()
    await page.locator('.browser-view .segmented-option', { hasText: 'Growth' }).click()

    const growth = page.getByTestId('archive-growth')
    await expect(growth.getByTestId('archive-growth-total')).toBeVisible({ timeout: 120_000 })
    await expect(growth).toContainText('Added at backup')

    // borg stores the absolute source path; folders that hold only one other
    // folder are merged, so the top level is the backed-up directory's var.
    const folders = growth.getByTestId('archive-growth-folders')
    const openFolder = async (name: RegExp): Promise<void> => {
      await folders.locator('button.growth-folder').filter({ hasText: name }).click()
    }
    await openFolder(/var$/)
    await openFolder(/^cache\/releases$/)
    await expect(folders).toContainText('app-2.4.0.tar')
    await expect(growth.locator('.path-crumbs')).toContainText('cache/releases')

    await growth.locator('.path-crumbs .crumb-root').click()
    await expect(folders.locator('button.growth-folder').first()).toHaveText(/var$/)

    await growth.locator('.segmented-option', { hasText: 'Largest files' }).click()
    await expect(
      growth.getByTestId('archive-growth-files').locator('tbody tr').first(),
    ).toContainText('var/cache/releases/app-2.4.0.tar')
  })
})
