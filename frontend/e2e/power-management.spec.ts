// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * Waking a host before a backup and powering it back down afterward, for
 * both the agent's own host and the repository host it backs up to.
 * media-store-01 and its media-weekly repository are the demo's seeded
 * "not always on" pair, so both already have wake/shutdown configured.
 */
test.describe('Power management', () => {
  test('agent power settings show the seeded values and can be edited', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/media-store-01?tab=settings&section=power')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('Host power')
    await expect(pane).toContainText('3C:97:0E:2B:9A:44')
    await expect(pane).toContainText('Agent process')

    await pane.getByRole('button', { name: 'Edit' }).click()
    await expect(page.locator('#power-wake-mac')).toHaveValue('3C:97:0E:2B:9A:44')

    await page.locator('#power-wake-timeout').fill('300')
    await pane.getByRole('button', { name: 'Save' }).click()

    // Back in view mode with the new value, and it survives a reload.
    await expect(pane.getByRole('button', { name: 'Edit' })).toBeVisible()
    await expect(pane).toContainText('300 seconds')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.settings-pane')).toContainText('300 seconds')
  })

  test('repository power settings show the seeded values and can be edited', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.locator('.entity-card').filter({ hasText: 'media-weekly' }).first().click()
    await page.waitForLoadState('networkidle')

    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.locator('.settings-nav-item', { hasText: 'Power' }).click()
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('Wake host before backup')
    await expect(pane).toContainText('9C:B6:D0:1A:44:7F')

    await pane.getByRole('button', { name: 'Edit' }).click()
    await expect(page.locator('#repo-power-wake-mac')).toHaveValue('9C:B6:D0:1A:44:7F')

    await page.locator('#repo-power-wake-timeout').fill('360')
    await pane.getByRole('button', { name: 'Save' }).click()

    await expect(pane.getByRole('button', { name: 'Edit' })).toBeVisible()
    await expect(pane).toContainText('360 seconds')
  })

  /**
   * The per-schedule override, end to end: web-server-01 has wake details on
   * file but waking switched off, and exactly one seeded schedule wakes it
   * anyway. Its power pane names that schedule, the link lands on the
   * schedule's own Power section, and the read-out there says the host is
   * woken for this job only.
   */
  test('a host names the schedules that wake it, and the link lands on the override', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01?tab=settings&section=power')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    // The wake details stay on screen even though this host does not wake by
    // default - a schedule still wakes it with them.
    await expect(pane).toContainText('A4:BB:6D:1F:22:8E')
    await expect(pane).toContainText('schedule wakes this host')

    await pane.locator('.override-link').first().click()
    await page.waitForLoadState('networkidle')

    const power = page.locator('.settings-pane')
    await expect(power.locator('.segmented-option[aria-checked="true"]')).toHaveText('Enabled')
    await expect(power).toContainText('Woken for this job only')
    await expect(power).toContainText('web-server-01')
  })

  test('a schedule can be switched to never waking its hosts, and it sticks', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01?tab=settings&section=power')
    await page.waitForLoadState('networkidle')
    await page.locator('.settings-pane .override-link').first().click()
    await page.waitForLoadState('networkidle')

    const power = page.locator('.settings-pane')
    await power.locator('.segmented-option', { hasText: 'Disabled' }).click()
    await expect(power.getByText('Not woken').first()).toBeVisible()

    await page.getByRole('button', { name: 'Save changes' }).click()
    await expect(page.locator('.save-success')).toBeVisible()

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.settings-pane .segmented-option[aria-checked="true"]')).toHaveText(
      'Disabled',
    )

    // Put it back, so the host's own pane keeps naming this schedule for
    // whichever spec runs next.
    await page.locator('.settings-pane .segmented-option', { hasText: 'Enabled' }).click()
    await page.getByRole('button', { name: 'Save changes' }).click()
    await expect(page.locator('.save-success')).toBeVisible()
  })
})
