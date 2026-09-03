// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * Staging a host's libvirt domains before a backup. db-server-01 is the
 * demo's virtualization host: staging is on, the domains a scan reported are
 * seeded, and its hourly schedule opts in.
 */
test.describe('Virtual machine staging', () => {
  test('the host settings and its domains are listed', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('/srv/vm-staging')
    await expect(pane).toContainText('7 increments')

    // One row per domain, with the mode the agent decided and what it staged.
    await expect(pane.locator('tbody tr')).toHaveCount(5)
    await expect(pane).toContainText('web01')
    await expect(pane).toContainText('Incremental')
    await expect(pane).toContainText('full + 4 increments')
    await expect(pane).toContainText('Excluded')
  })

  test('the host settings can be edited', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await pane.getByRole('button', { name: 'Edit' }).click()
    await expect(page.locator('#vm-staging-dir')).toHaveValue('/srv/vm-staging')

    await page.locator('#vm-full-interval').fill('14')
    await pane.getByRole('button', { name: 'Save' }).click()

    await expect(pane.getByRole('button', { name: 'Edit' })).toBeVisible()
    await expect(pane).toContainText('14 increments')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.settings-pane')).toContainText('14 increments')
  })

  test('a per-domain limit is saved on its own', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=vms')
    await page.waitForLoadState('networkidle')

    const row = page.locator('tbody tr', { hasText: 'web01' })
    await expect(row).toContainText('Host default')

    await row.locator('input.vm-limit').fill('300')
    await row.locator('input.vm-limit').blur()
    await expect(row).toContainText('Overridden')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('tbody tr', { hasText: 'web01' })).toContainText('Overridden')
  })

  test('a schedule opts in from its advanced settings', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await page.locator('.entity-card', { hasText: 'db-server-01' }).first().click()
    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.locator('.settings-nav-item', { hasText: 'Advanced' }).click()

    await expect(page.locator('.settings-pane')).toContainText('Stage virtual machines')
  })
})
