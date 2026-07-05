// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Settings journey', () => {
  test('global excludes page loads with common patterns', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/excludes')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Global Excludes' })).toBeVisible()
    await expect(page.getByRole('textbox')).toBeVisible()

    const patterns = page.getByRole('textbox')
    const value = await patterns.inputValue()
    expect(value).toContain('node_modules')
    expect(value).toContain('__pycache__')
  })

  test('notifications page shows configured channels', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/notifications')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Notifications' })).toBeVisible()
    await expect(page.getByText('Ops Webhook')).toBeVisible()
    await expect(page.getByText('Admin Email')).toBeVisible()
    await expect(page.getByText('Webhook', { exact: true })).toBeVisible()
    await expect(page.getByText('Email', { exact: true })).toBeVisible()

    const toggles = page.getByRole('switch')
    await expect(toggles.first()).toBeChecked()
  })

  test('tunnels page shows configured tunnel connection details', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/tunnels')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Tunnels' })).toBeVisible()
    await expect(page.getByText('Connected')).toBeVisible()
    await expect(page.getByText('127.0.0.1')).toBeVisible()
    await expect(page.getByText('borg')).toBeVisible()
  })

  test('settings submenu contains excludes link', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await page.getByRole('button', { name: 'Settings' }).click()
    await expect(page.getByRole('link', { name: 'Excludes' })).toBeVisible()
  })
})
