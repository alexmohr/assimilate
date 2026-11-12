// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, openSeededComparisonArchive, test } from './fixtures'

/*
 * start-agent.sh gives web-server-01's two newest archives a different
 * logo.png (a 24x16 green image in place of a 16x16 blue one), so comparing
 * the newest archive with an older one lists the logo as changed. Opening it
 * reads both versions through the single-file extract endpoint and shows them
 * side by side, swiped or faded.
 */
test.describe('Comparing an image between archives', () => {
  test('a changed image opens in the image comparison, in every mode', async ({ page }) => {
    const { base } = await openSeededComparisonArchive(page)
    const card = page.locator('.browser-panel').last()

    await card.getByRole('tab', { name: 'Changes' }).click()
    await card.getByRole('button', { name: 'Other archive' }).click()
    await card.getByLabel('Archive to compare with').selectOption(base)

    // Listing the changes runs a borg diff, which can take a while.
    const logo = card.getByRole('button', { name: /^Compare .*logo\.png$/ })
    await expect(logo).toBeVisible({ timeout: 120_000 })
    await expect(logo).toContainText('compare images')
    await logo.click()

    const dialog = page.getByRole('dialog', { name: 'Compare logo.png' })
    await expect(dialog).toBeVisible()

    // 2-up: both versions, each read back from its own archive.
    await expect(dialog.getByAltText('logo.png before')).toBeVisible({ timeout: 60_000 })
    await expect(dialog.getByAltText('logo.png after')).toBeVisible()
    await expect(dialog.locator('.image-diff-summary')).toContainText('16 x 16')
    await expect(dialog.locator('.image-diff-summary')).toContainText('24 x 16')

    await dialog.getByRole('radio', { name: 'Swipe' }).click()
    await expect(
      dialog.getByRole('slider', { name: 'How much of the new image to show' }),
    ).toBeVisible()

    await dialog.getByRole('radio', { name: 'Onion skin' }).click()
    await expect(dialog.getByRole('slider', { name: 'Opacity of the new image' })).toBeVisible()

    await dialog.getByRole('button', { name: 'Close' }).click()
    await expect(dialog).not.toBeVisible()
  })
})
