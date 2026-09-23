// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, scheduleIdByName, test } from './fixtures'

/**
 * A Run now or Retry for a host that is offline is held until the agent
 * reconnects. The seed leaves one such run behind on offline-due-01, which
 * never connects in the demo (see `.devcontainer/demo/seed-demo.sh`), so the
 * pages that show it can be checked without waiting on a real outage.
 */
test.describe('a run queued for an offline host', () => {
  test('the schedule reads Queued and names the host it waits for', async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Queued run demo')
    await page.goto(`/schedules/${id}`)
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.badge', { hasText: 'Queued' })).toBeVisible()
    await expect(page.locator('.badge', { hasText: 'Running' })).toHaveCount(0)

    const card = page.locator('.live-log-card')
    await expect(card).toContainText('Backup queued')
    await expect(card).toContainText('is offline. The backup starts as soon as it reconnects.')
    await expect(card).not.toContainText('Waiting for progress')
    // Still called off the same way a running backup is.
    await expect(page.getByRole('button', { name: 'Cancel backup' })).toBeVisible()
  })

  test("the offline agent's overview shows the backup queued, not in progress", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/offline-due-01')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.live-log-card')
    await expect(card).toContainText('Backup queued')
    await expect(card).not.toContainText('Backup in progress')
  })
})
