// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Admin journey', () => {
  test('users list shows all seeded users with roles', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/users')
    await page.waitForLoadState('networkidle')

    await expect(page.getByText('operator1')).toBeVisible()
    await expect(page.getByText('viewer1')).toBeVisible()
    await expect(page.getByText('admin').first()).toBeVisible()
    await expect(page.getByText('operator').first()).toBeVisible()
    await expect(page.getByText('viewer').first()).toBeVisible()
  })

  test('groups page shows seeded groups', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/admin/groups')
    await page.waitForLoadState('networkidle')

    await expect(page.getByText('backend-team')).toBeVisible()
    await expect(page.getByText('data-team')).toBeVisible()
  })

  test('audit log page shows events with recognizable actions', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/audit-log')
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/audit-log/)

    const eventRows = page.locator('.audit-table tbody tr')
    await expect(eventRows.first()).toBeVisible()

    const badges = page.locator('.badge')
    await expect(badges.first()).toBeVisible()
    const badgeText = await badges.allInnerTexts()
    const hasExpectedAction = badgeText.some(
      (t) =>
        t.toLowerCase().includes('create') ||
        t.toLowerCase().includes('login') ||
        t.toLowerCase().includes('update'),
    )
    expect(hasExpectedAction).toBe(true)
  })
})
