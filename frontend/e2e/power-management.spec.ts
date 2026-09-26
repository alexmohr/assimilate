// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Locator } from '@playwright/test'

/**
 * Waking a host before a backup and powering it back down afterward, for
 * both the agent's own host and the repository host it backs up to.
 * media-store-01 and the host its media-weekly repository lives on are the
 * demo's seeded "not always on" pair, so both already have wake/shutdown
 * configured.
 */
/**
 * The power card's own Edit button. The Power pane also carries the "When the
 * host is offline" section, which has an Edit button of its own below this one.
 */
function powerEdit(pane: Locator): Locator {
  return pane.getByRole('button', { name: 'Edit' }).first()
}

test.describe('Power management', () => {
  test('agent power settings show the seeded values and can be edited', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/media-store-01?tab=settings&section=power')
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('Host power')
    await expect(pane).toContainText('3C:97:0E:2B:9A:44')
    await expect(pane).toContainText('Agent process')

    await powerEdit(pane).click()
    await expect(page.locator('#power-wake-mac')).toHaveValue('3C:97:0E:2B:9A:44')

    await page.locator('#power-wake-timeout').fill('300')
    await pane.getByRole('button', { name: 'Save' }).first().click()

    // Back in view mode with the new value, and it survives a reload.
    await expect(powerEdit(pane)).toBeVisible()
    await expect(pane).toContainText('300 seconds')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.settings-pane')).toContainText('300 seconds')
  })

  /**
   * Power belongs to the repository host: the repository shows its host's
   * settings read-only and links there, and a change made on the host reads
   * back on every repository on it.
   */
  test('repository power settings are shown on the repository and edited on its host', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.locator('.entity-card').filter({ hasText: 'media-weekly' }).first().click()
    await page.waitForLoadState('networkidle')

    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.locator('.settings-nav-item', { hasText: 'Power' }).click()
    await page.waitForLoadState('networkidle')

    const repoPane = page.locator('.settings-pane')
    await expect(repoPane).toContainText('Wake host before backup')
    await expect(repoPane).toContainText('9C:B6:D0:1A:44:7F')
    await expect(repoPane.getByRole('button', { name: 'Edit' })).toHaveCount(0)

    await repoPane.getByRole('link', { name: 'Edit on host' }).click()
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(/\/repo-hosts\/\d+\?section=power/)

    const hostPane = page.locator('.settings-pane')
    await powerEdit(hostPane).click()
    await expect(page.locator('#repo-host-power-wake-mac')).toHaveValue('9C:B6:D0:1A:44:7F')

    await page.locator('#repo-host-power-wake-timeout').fill('360')
    await hostPane.getByRole('button', { name: 'Save' }).first().click()

    await expect(powerEdit(hostPane)).toBeVisible()
    await expect(hostPane).toContainText('360 seconds')

    // database-hourly lives on the same host, so it reads the same setting.
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.locator('.entity-card').filter({ hasText: 'database-hourly' }).first().click()
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.locator('.settings-nav-item', { hasText: 'Power' }).click()
    await expect(page.locator('.settings-pane')).toContainText('360 seconds')
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

    // Scoped to the view that owns it: following the link below leaves both
    // detail views briefly mounted, and a bare `.settings-pane` then matches
    // two elements, which Playwright's strict mode rejects.
    const pane = page.locator('.host-detail .settings-pane')
    // The wake details stay on screen even though this host does not wake by
    // default - a schedule still wakes it with them.
    await expect(pane).toContainText('A4:BB:6D:1F:22:8E')
    // Count-independent: the demo seeds one schedule that overrides this host,
    // but the specs share one demo instance and others create schedules of
    // their own, so the note's singular/plural wording is not ours to pin.
    await expect(pane).toContainText('whatever the setting above says')
    await expect(pane.locator('.override-link').first()).toBeVisible()

    await pane.locator('.override-link').first().click()
    // The route swap resolves after `networkidle` does, so wait on the URL
    // rather than on the network going quiet.
    await page.waitForURL(/\/schedules\/\d+/)
    await page.waitForLoadState('networkidle')

    const power = page.locator('.schedule-detail .settings-pane')
    await expect(power.locator('.segmented-option[aria-checked="true"]')).toHaveText('Enabled')
    await expect(power).toContainText('Woken for this job only')
    await expect(power).toContainText('web-server-01')
  })
})
