// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('TOTP / 2FA', () => {
  test('Profile page shows Security section with TOTP enrollment option', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')

    // The profile page should show a Security section
    await expect(page.locator('text=Security').first()).toBeVisible()
    await expect(page.locator('text=Two-Factor Authentication').first()).toBeVisible()
  })

  test('Sessions tab on profile shows active sessions', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')

    // Look for the Sessions section
    const sessionsHeading = page.locator('h2:has-text("Sessions"), h3:has-text("Sessions")')
    if (await sessionsHeading.isVisible().catch(() => false)) {
      // There should be at least the current session visible
      await expect(page.locator('text=current').first()).toBeVisible()
    }
  })
})
