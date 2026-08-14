// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Route } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Repositories management journey', () => {
  test('repo list page shows known demo repositories', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const repoCards = page.locator('.repo-card')
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

    const firstCard = page.locator('.repo-card').first()
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

    const chip = page.locator('.repo-card .entity-issue-chip.sev-warning').first()
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

    await expect(page.locator('.dialog-title')).toHaveText('Break Repository Lock')
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

    await page.getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.locator('.dialog-title')).not.toBeVisible()
  })
})
