// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Route } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Repositories management journey', () => {
  test('repo list page shows known demo repositories', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const repoCards = page.locator('.entity-card')
    await expect(repoCards.first()).toBeVisible()

    const text = await page.locator('body').innerText()
    const hasRepo =
      text.includes('server-daily') ||
      text.includes('database-hourly') ||
      text.includes('media-weekly') ||
      text.includes('lz4') ||
      text.includes('zstd') ||
      text.includes('repokey')
    expect(hasRepo).toBe(true)
  })

  test('repo detail page shows compression and encryption info', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1')
    await page.waitForLoadState('networkidle')

    const text = await page.locator('body').innerText()

    const hasCompression = text.includes('lz4') || text.includes('zstd') || text.includes('none')
    expect(hasCompression).toBe(true)

    const hasEncryption =
      text.includes('repokey') ||
      text.includes('blake2') ||
      text.includes('authenticated') ||
      text.includes('none') ||
      text.includes('encryption') ||
      text.includes('Encryption')
    expect(hasEncryption).toBe(true)
  })

  test('clicking a repo from the list navigates to detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const firstCard = page.locator('.entity-card').first()
    await expect(firstCard).toBeVisible()
    await firstCard.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repos\/\d+/)
  })

  test('repo detail shows associated schedules or archives info', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1')
    await page.waitForLoadState('networkidle')

    const text = await page.locator('body').innerText()
    const hasRelatedInfo =
      text.includes('schedule') ||
      text.includes('Schedule') ||
      text.includes('archive') ||
      text.includes('Archive') ||
      text.includes('backup') ||
      text.includes('Backup')
    expect(hasRelatedInfo).toBe(true)
  })

  test('repo card shows a clickable unmatched chip that navigates to the archives tab', async ({
    page,
  }) => {
    await page.route('**/api/repos/stats', async (route: Route) => {
      const response = await route.fetch()
      const repos = (await response.json()) as Array<Record<string, unknown>>
      if (repos.length > 0) repos[0].unmatched_count = 3
      return route.fulfill({
        status: response.status(),
        contentType: 'application/json',
        body: JSON.stringify(repos),
      })
    })

    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const chip = page.locator('.entity-card .entity-issue-chip.sev-warning').first()
    await expect(chip).toBeVisible()
    await expect(chip).toContainText('unmatched')

    await chip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repos\/\d+\?tab=archives/)
  })

  test('breaking a repository lock shows a success result in the confirmation dialog', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1')
    await page.waitForLoadState('networkidle')

    const dangerZone = page.locator('.danger-zone')
    const breakLockBtn = dangerZone.getByRole('button', { name: 'Break Lock', exact: true })
    await expect(breakLockBtn).toBeVisible()
    await breakLockBtn.click()

    await expect(page.locator('.modal-title')).toHaveText('Break Repository Lock')
    await expect(page.locator('.break-lock-warning').first()).toContainText(
      'stale local cache lock',
    )

    await page.getByRole('button', { name: 'Yes, Break Lock', exact: true }).click()

    // Demo repo has no active lock, so borg break-lock is a safe no-op that
    // still reports success - the exact wording isn't asserted since it's
    // borg's own message, just that the dialog reflects a result and not an
    // error.
    await expect(page.locator('.break-lock-success')).toBeVisible({ timeout: 15_000 })
    await expect(page.locator('.form-error')).not.toBeVisible()

    // Scoped to the footer: BaseModal's own dismiss control also exposes the
    // accessible name "Close" (F-22 gave it one), so an unscoped lookup is
    // ambiguous. This targets the same footer button the test always meant.
    await page.locator('.modal-footer').getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.locator('.modal-title')).not.toBeVisible()
  })

  test('the Repositories page groups by host by default with a shared storage pool', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    // The quota filter chips are always visible: All, At risk, No quota.
    await expect(page.locator('.quota-fchip')).toHaveCount(3)
    await expect(page.locator('.quota-fchip', { hasText: 'All' })).toBeVisible()

    // The demo's server-daily/database-hourly/media-weekly repos share the "localhost"
    // ssh_host and a configured server quota (see .devcontainer/demo/seed-demo.sh),
    // and group by host is the default view - no need to toggle it on.
    const poolHeader = page.locator('.pool-header', { hasText: 'localhost' })
    await expect(poolHeader).toBeVisible()
    await expect(poolHeader.locator('.pool-track')).toBeVisible()

    // Each demo repo also has its own quota configured, so its card shows a usage bar
    // on its own scale rather than the shared pool scale.
    await expect(page.locator('.entity-card .quota-meter').first()).toBeVisible()

    // media-weekly's demo quota (warn_bytes: 1) is always in a breached state, so it
    // should still be visible as an at-risk repo once the filter narrows the view.
    await page.locator('.quota-fchip', { hasText: 'At risk' }).click()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.entity-card', { hasText: 'media-weekly' })).toBeVisible()
  })
})
