// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('repo archives grouped by host name each group, not just the count', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await page.getByText('server-daily').click()
  await page.waitForURL(/\/repos\/\d+/)

  await page.getByRole('tab', { name: 'Archives' }).click()
  await page.waitForURL(/tab=archives/)

  const group = page.locator('.archive-group').first()
  await expect(group).toBeVisible()

  const hostname = group.locator('.group-hostname')
  await expect(hostname).toBeVisible()
  await expect(hostname).not.toBeEmpty()
  await expect(group.locator('.group-count')).toBeVisible()
  // The summed size is what makes a collapsed group worth reading.
  await expect(group.locator('.group-size')).not.toBeEmpty()
})
