// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'

test.describe('Profile theme', () => {
  test('the lighter dark themes apply at once and survive a reload', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/profile')
    const appearance = await openSettingsSection(page, 'Appearance')
    const html = page.locator('html')

    await appearance.getByRole('button', { name: 'Charcoal' }).click()
    await expect(html).toHaveClass(/\bdark\b/)
    await expect(html).toHaveClass(/\btheme-charcoal\b/)

    await appearance.getByRole('button', { name: 'Warm' }).click()
    await expect(html).toHaveClass(/\btheme-warm\b/)
    await expect(html).not.toHaveClass(/\btheme-charcoal\b/)

    // The choice is stored on the server, so a fresh page load picks it up.
    await page.reload()
    await expect(html).toHaveClass(/\btheme-warm\b/)
    const stored = await page.request.get('/api/auth/preferences')
    expect((await stored.json()).theme).toBe('warm')

    // Back to following the system (light in the test browser), so the
    // account is left as it was found.
    const again = await openSettingsSection(page, 'Appearance')
    await again.getByRole('button', { name: 'Auto' }).click()
    await expect(html).not.toHaveClass(/\bdark\b/)
    await expect(html).not.toHaveClass(/\btheme-warm\b/)
  })
})
