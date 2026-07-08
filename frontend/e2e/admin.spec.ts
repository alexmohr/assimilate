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

  test('user edit permissions tab shows repository names', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/users')
    await page.waitForLoadState('networkidle')

    const operatorRow = page.locator('tr', { hasText: 'operator1' })
    await operatorRow.getByRole('button', { name: 'Edit' }).click()

    await page.getByRole('button', { name: 'Permissions' }).click()

    const repoCell = page.locator('.perm-repo-cell').first()
    await expect(repoCell).toBeVisible()
    await expect(repoCell).not.toHaveText('')
    await expect(repoCell).not.toHaveText('/')
    await expect(page.getByText('server-daily')).toBeVisible()
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
