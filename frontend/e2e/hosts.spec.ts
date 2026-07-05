// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Route } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Hosts management', () => {
  test('hosts list shows connected agent hosts and imported placeholders', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await expect(page.getByText('web-server-01', { exact: true })).toBeVisible()
    await expect(page.getByText('db-server-01', { exact: true })).toBeVisible()
    await expect(page.getByText('media-store-01', { exact: true })).toBeVisible()
    await expect(page.getByText('old-webserver', { exact: true })).toBeVisible()
    await expect(page.getByText('legacy-db-prod', { exact: true })).toBeVisible()
  })

  test('clicking a host navigates to its detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await page.locator('.host-card').filter({ hasText: 'web-server-01' }).first().click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\//)
    await expect(page.getByText('web-server-01').first()).toBeVisible()
  })

  test('deploy dialog opens and shows Load from remote button', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    // Demo agents have no agent_version so they show a Deploy button (not imported).
    const deployBtn = page
      .locator('.host-card')
      .filter({ hasText: 'web-server-01' })
      .locator('.card-actions button', { hasText: /Deploy|Upgrade/ })
      .first()
    await expect(deployBtn).toBeVisible({ timeout: 15_000 })
    await deployBtn.click()

    await expect(page.getByRole('heading', { name: /Deploy|Upgrade/ }).first()).toBeVisible()

    // The "Load from remote" button must be present - this was added in issue #124.
    const loadBtn = page.getByRole('button', { name: 'Load from remote' })
    await expect(loadBtn).toBeVisible()
    await expect(loadBtn).not.toBeDisabled()
  })

  test('agent card shows expandable CardError for failed backups', async ({ page }) => {
    // Intercept the health API to inject a failure with an error message for web-server-01.
    await page.route('**/api/stats/health', async (route: Route) => {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify([
          {
            hostname: 'web-server-01',
            target_name: 'server-daily',
            last_status: 'failed',
            last_backup_at: new Date().toISOString(),
            is_overdue: false,
            last_error_message: 'Repository lock could not be acquired',
          },
        ]),
      })
    })

    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.host-card').filter({ hasText: 'web-server-01' }).first()

    const errorToggle = card.locator('.error-toggle')
    await expect(errorToggle).toBeVisible()

    const errorPre = card.locator('.error-pre')
    await expect(errorPre).not.toBeVisible()

    await errorToggle.click()
    await expect(errorPre).toBeVisible()
    await expect(errorPre).toContainText('Repository lock could not be acquired')

    await errorToggle.click()
    await expect(errorPre).not.toBeVisible()
  })
})
