// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'

async function groupId(page: Page, name: string): Promise<number> {
  const groups = (await (await page.request.get('/api/groups')).json()) as {
    id: number
    name: string
  }[]
  const id = groups.find((group) => group.name === name)?.id
  expect(id, `seeded group "${name}" must exist`).toBeDefined()
  return id ?? 0
}

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

  test("a group's repository permissions can be set and are kept", async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/admin/groups')
    await page.waitForLoadState('networkidle')

    // The demo grants data-team View and Backup on database-hourly.
    const row = page.locator('tr', { hasText: 'data-team' })
    await row.getByRole('button', { name: 'Permissions' }).click()
    const dialog = page.getByRole('dialog', { name: 'Group repository permissions' })
    const extract = dialog.getByRole('checkbox', { name: 'Extract on database-hourly' })
    await expect(dialog.getByRole('checkbox', { name: 'View on database-hourly' })).toBeChecked()
    await expect(extract).not.toBeChecked()

    await extract.check()
    await dialog.getByRole('button', { name: 'Save permissions' }).click()
    await expect(dialog).toBeHidden()

    const grants = (await (
      await page.request.get(`/api/groups/${await groupId(page, 'data-team')}/permissions`)
    ).json()) as { permissions: string[] }[]
    expect(grants.flatMap((grant) => grant.permissions)).toEqual(['view', 'backup', 'extract'])

    // Put the seeded grant back for the other tests.
    await row.getByRole('button', { name: 'Permissions' }).click()
    await expect(extract).toBeChecked()
    await extract.uncheck()
    await dialog.getByRole('button', { name: 'Save permissions' }).click()
    await expect(dialog).toBeHidden()
  })

  test('roles offer View All Agents, which the operator holds', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/admin/roles')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('columnheader', { name: 'View All Agents' })).toBeVisible()
    const operator = page.locator('tr', { hasText: 'operator' })
    await expect(operator.locator('td[data-label="View All Agents"] .perm-yes')).toBeVisible()
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
