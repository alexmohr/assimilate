// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expandAllArchiveGroups, expect, loginAsAdmin, test } from './fixtures'

test.describe('transport failure retry', () => {
  test('archive list loads after its first request is reset by the network', async ({ page }) => {
    await loginAsAdmin(page)

    // The first list request dies without any HTTP response, as a reset
    // keep-alive connection does; later ones reach the server.
    let attempts = 0
    await page.route('**/api/repos/1/archives', (route) => {
      attempts += 1
      return attempts === 1 ? route.abort('connectionreset') : route.continue()
    })

    await page.goto('/repos/1?tab=archives')
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    await expect(page.locator('.archive-row').first()).toBeVisible({ timeout: 30_000 })
    expect(attempts).toBeGreaterThanOrEqual(2)
  })
})
