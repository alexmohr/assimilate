// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, interceptScheduleSave, loginAsAdmin, scheduleIdByName, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * Filesystem snapshots (docs/filesystem-snapshots.md). seed-demo.sh sets the
 * "Web server dual-target" schedule to snapshot its sources if possible,
 * with 15% LVM snapshots, and gives its newest run on "demo" the snapshot's
 * steps on the run timeline.
 */
test.describe('Filesystem snapshots', () => {
  async function chooseAndSave(
    page: Page,
    scheduleId: number,
    mode: string,
    percent: number,
  ): Promise<Record<string, unknown>> {
    const pane = page.locator('.settings-pane')
    await pane
      .getByRole('radiogroup', { name: 'Filesystem snapshots' })
      .getByRole('radio', { name: mode })
      .click()
    await pane.locator('#schedule-lvm-snapshot-percent').fill(String(percent))
    const waitForSave = await interceptScheduleSave(
      page,
      scheduleId,
      (_request, response) => response,
    )
    await page.getByRole('button', { name: 'Save changes' }).click()
    // A later intercept takes precedence over this one, so each save is seen
    // by its own.
    return waitForSave()
  }

  test('a schedule chooses its snapshot mode and LVM snapshot size', async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Web server dual-target')
    await page.goto(`/schedules/${id}?tab=settings&section=advanced`)
    await page.waitForLoadState('networkidle')

    const pane = page.locator('.settings-pane')
    const modes = pane.getByRole('radiogroup', { name: 'Filesystem snapshots' })
    await expect(modes.getByRole('radio', { name: 'If possible' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await expect(pane.locator('#schedule-lvm-snapshot-percent')).toHaveValue('15')

    const saved = await chooseAndSave(page, id, 'Required', 20)
    expect(saved).toMatchObject({ fs_snapshot: 'required', lvm_snapshot_percent: 20 })

    // Put the seeded settings back for the screenshots and the other tests.
    const restored = await chooseAndSave(page, id, 'If possible', 15)
    expect(restored).toMatchObject({ fs_snapshot: 'best_effort', lvm_snapshot_percent: 15 })

    // Off has no snapshot to size.
    await modes.getByRole('radio', { name: 'Off' }).click()
    await expect(pane.locator('#schedule-lvm-snapshot-percent')).toHaveCount(0)
  })

  test("a run's snapshot steps are on its timeline", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Web server dual-target')
    await page.goto(`/schedules/${id}`)
    await page.waitForLoadState('networkidle')

    // The schedule's newest copy on server-daily is the seeded snapshot run;
    // a run the agent page's Logs tab used to list is opened from the
    // schedule's Recent runs now. Other specs can add runs after it, so each
    // of server-daily's runs is opened from the newest until one shows the
    // snapshot, with its host rows expanded.
    const taken = page
      .locator('.runs-detail')
      .getByText('Snapshotted LVM volume vg0/www (/var/www)')
      .first()
    const strip = page.locator('.runs-row').filter({ hasText: 'server-daily' })
    const pills = strip.locator('.run-pill')
    await expect(pills.first()).toBeVisible()
    for (let i = (await pills.count()) - 1; i >= 0; i--) {
      await pills.nth(i).click()
      const rows = page.locator('.runs-detail .log-row-summary--button')
      for (let r = 0; r < (await rows.count()); r++) {
        if ((await rows.nth(r).getAttribute('aria-expanded')) !== 'true') await rows.nth(r).click()
      }
      const shown = await taken
        .waitFor({ state: 'visible', timeout: 5_000 })
        .then(() => true)
        .catch(() => false)
      if (shown) break
    }

    await expect(taken, 'the seeded snapshot run is in Recent runs').toBeVisible()
    await expect(
      page.getByText(/^LVM snapshot vg0\/assimilate-www-schedule-/).first(),
    ).toBeVisible()
  })
})
