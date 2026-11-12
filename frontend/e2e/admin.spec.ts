// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'

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

  test("a user's repository permissions show repository names", async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/users')
    await page.waitForLoadState('networkidle')

    // A user's row opens their own page, which replaced the edit dialog.
    await page.locator('.list-row[data-user="operator1"]').click()
    await expect(page).toHaveURL(/\/users\/\d+/)

    const permissions = await openSettingsSection(page, 'Repository permissions')
    const repoCell = permissions.locator('tbody td.cell-mono').first()
    await expect(repoCell).toBeVisible()
    await expect(repoCell).not.toHaveText('')
    await expect(repoCell).not.toHaveText('/')
    await expect(permissions.getByText('server-daily', { exact: true })).toBeVisible()
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

    const eventRows = page.getByTestId('audit-entries').locator('.log-row')
    await expect(eventRows.first()).toBeVisible()

    // Each row names its action in words under the user. The demo seed
    // records real audited actions (see seed-demo.sh).
    const actions = await eventRows.locator('.log-row-subtitle').allTextContents()
    const hasExpectedAction = actions.some((t) =>
      ['Key export', 'Delete archive', 'Restore files'].includes(t.trim()),
    )
    expect(hasExpectedAction).toBe(true)
  })

  test('audit log records sign-ins and access changes', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/audit-log')
    await page.waitForLoadState('networkidle')

    const actionFilter = page.locator('.action-filter')
    const apply = page.getByRole('button', { name: 'Apply' })
    const firstRow = page.getByTestId('audit-entries').locator('.log-row').first()

    // The login this test just made is itself audited.
    await actionFilter.fill('login')
    await apply.click()
    await expect(firstRow.locator('.log-row-subtitle')).toHaveText('Login')
    await expect(firstRow.locator('.log-row-title')).toHaveText('admin')
    await expect(firstRow.locator('.log-row-facts')).toContainText('user')

    // The demo seed records a role change with what it was before and after.
    await actionFilter.fill('set_user_roles')
    await apply.click()
    await expect(firstRow.locator('.log-row-subtitle')).toHaveText('Set user roles')
    await firstRow.locator('.log-row-summary').click()
    const details = firstRow.locator('.detail-pre').first()
    await expect(details).toContainText('"before"')
    await expect(details).toContainText('"after"')
    await expect(details).toContainText('operator1')
  })
})
