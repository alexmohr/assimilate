// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, interceptScheduleSave, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * The repository half of catch-up: the two fields that configure it, and the
 * live block that says what is being waited on.
 *
 * The seed writes a pending repository marker for "Catch-up on an offline
 * repository demo" (see `.devcontainer/demo/seed-demo.sh`), because waiting out
 * a real outage against a host that never answers is not something an e2e run
 * can do.
 */
test.describe('repository catch-up', () => {
  async function openGeneralSettings(page: Page, scheduleId: number): Promise<void> {
    await page.goto(`/schedules/${scheduleId}`)
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.getByRole('button', { name: 'General' }).click()
  }

  test('the re-check interval and give-up window save together', async ({ page }) => {
    await loginAsAdmin(page)
    await openGeneralSettings(page, 1)

    // Both belong to catch-up, so neither exists until it is switched on.
    await expect(page.locator('#catch-up-recheck')).toHaveCount(0)
    await expect(page.locator('#catch-up-give-up')).toHaveCount(0)

    await page.getByRole('switch', { name: 'Catch up missed runs' }).click()

    const recheck = page.locator('#catch-up-recheck')
    await expect(recheck).toHaveValue('15')
    await recheck.fill('30')

    // Empty is the "wait indefinitely" default, so there is nothing to clear.
    const giveUp = page.locator('#catch-up-give-up')
    await expect(giveUp).toHaveValue('')
    await page.getByLabel('Give-up window unit').selectOption('days')
    await giveUp.fill('3')

    const waitForSave = await interceptScheduleSave(page, 1, (requestBody, responseBody) => ({
      ...responseBody,
      catch_up_missed_runs: requestBody.catch_up_missed_runs,
      catch_up_repo_recheck_minutes: requestBody.catch_up_repo_recheck_minutes,
      catch_up_give_up_minutes: requestBody.catch_up_give_up_minutes,
    }))

    await page.getByRole('button', { name: 'Save changes' }).click()

    const saved = await waitForSave()
    expect(saved.catch_up_missed_runs).toBe(true)
    expect(saved.catch_up_repo_recheck_minutes).toBe(30)
    // Three days, stored as the minutes the column holds.
    expect(saved.catch_up_give_up_minutes).toBe(4320)
  })

  test('a pending wait names its repository and can be re-checked on demand', async ({ page }) => {
    await loginAsAdmin(page)

    const scheduleId = await page.evaluate(async () => {
      const response = await fetch('/api/schedules', { credentials: 'include' })
      const rows = (await response.json()) as { id: number; name: string }[]
      return rows.find((r) => r.name === 'Catch-up on an offline repository demo')?.id ?? 0
    })
    expect(scheduleId).toBeGreaterThan(0)

    await openGeneralSettings(page, scheduleId)

    const waiting = page.locator('.agent-row', { hasText: 'server-daily' })
    await expect(waiting).toBeVisible()
    await expect(waiting).toContainText('last checked')
    await expect(waiting).toContainText('next check')
    // The seed gives this one a three-day window, so it counts down rather than
    // waiting forever.
    await expect(waiting).toContainText('giving up')

    // The demo's repository host does answer, so the check reports back rather
    // than leaving the button spinning.
    await page.getByRole('button', { name: 'Check now' }).click()
    await expect(page.locator('.toast, .p-toast-message').first()).toBeVisible({ timeout: 15_000 })
  })
})
