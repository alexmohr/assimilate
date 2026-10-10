// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Admin journey', () => {
  test('users list shows all seeded users with roles', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/users')
    await page.waitForLoadState('networkidle')

    await expect(page.getByText('operator1', { exact: true })).toBeVisible()
    await expect(page.getByText('viewer1', { exact: true })).toBeVisible()
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

    await page.getByRole('tab', { name: 'Permissions' }).click()

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
    // The demo seed records real audited actions (see seed-demo.sh). Read the
    // DOM text rather than innerText: the badge's CSS upper-cases what it shows.
    const badgeText = await badges.allTextContents()
    const hasExpectedAction = badgeText.some((t) =>
      ['key_export', 'delete_archive', 'restore_files'].includes(t.trim()),
    )
    expect(hasExpectedAction).toBe(true)
  })

  test('audit log records sign-ins and access changes', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/audit-log')
    await page.waitForLoadState('networkidle')

    const actionFilter = page.locator('.action-filter')
    const apply = page.getByRole('button', { name: 'Apply' })
    const firstRow = page.locator('.audit-table tbody tr').first()

    // The login this test just made is itself audited.
    await actionFilter.fill('login')
    await apply.click()
    await expect(firstRow.locator('.badge')).toHaveText('login')
    await expect(firstRow.locator('.cell-user')).toHaveText('admin')
    await expect(firstRow.locator('.cell-target')).toContainText('user')

    // The demo seed records a role change with what it was before and after.
    await actionFilter.fill('set_user_roles')
    await apply.click()
    await expect(firstRow.locator('.badge')).toHaveText('set_user_roles')
    await firstRow.getByRole('button').click()
    const details = page.locator('.detail-pre').first()
    await expect(details).toContainText('"before"')
    await expect(details).toContainText('"after"')
    await expect(details).toContainText('operator1')
  })
})
