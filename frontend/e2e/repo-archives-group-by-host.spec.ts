// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('repo archives grouped by host shows the hostname link, not just the count', async ({
  page,
}) => {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await page.getByText('server-daily').click()
  await page.waitForURL(/\/repos\/\d+/)

  await page.getByRole('button', { name: 'Archives' }).click()
  await page.waitForURL(/tab=archives/)

  const group = page.locator('.archive-group').first()
  await expect(group).toBeVisible()

  const hostLink = group.locator('.group-hostname')
  await expect(hostLink).toBeVisible()
  await expect(hostLink).not.toBeEmpty()
  await expect(group.locator('.group-count')).toBeVisible()
})
