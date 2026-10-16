// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Locator, Page } from '@playwright/test'
import { expect, loginAsAdmin, seededIdByName, test } from './fixtures'

/**
 * A dependency's Used by pane: every schedule and agent that needs it, and
 * where that need comes from.
 *
 * The demo's nas-media is required by "Media share nightly" on media-store-01
 * itself; files-01 is required by media-store-01's backup defaults and by the
 * "localhost" repository host (see `.devcontainer/demo/seed-demo.sh`).
 */
async function openUsedBy(page: Page, name: string): Promise<{ id: number; rows: Locator }> {
  const id = await seededIdByName(page, '/api/dependency-hosts', name)
  await page.goto(`/dependency-hosts/${id}?section=used-by`)
  await page.waitForLoadState('networkidle')
  return { id, rows: page.locator('a.agent-row') }
}

test.describe('Dependency Used by', () => {
  test('lists the schedule that requires it, linking to that schedule', async ({ page }) => {
    await loginAsAdmin(page)
    const { rows } = await openUsedBy(page, 'nas-media')

    const row = rows.filter({ hasText: 'Media share nightly' })
    await expect(row).toHaveCount(1)
    await expect(row.locator('.meta-pill')).toHaveText('media-store-01')
    await expect(row.locator('.agent-row-stats')).toHaveText('set on the schedule')

    const scheduleId = await seededIdByName(page, '/api/schedules', 'Media share nightly')
    await row.click()
    await page.waitForURL(`**/schedules/${scheduleId}`)
  })

  test("says when the need comes from an agent's defaults or a repository host", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    const { rows } = await openUsedBy(page, 'files-01')

    const fromAgent = rows.filter({ hasText: 'from agent defaults' })
    await expect(fromAgent.first()).toBeVisible()
    await expect(fromAgent.locator('.meta-pill').first()).toHaveText('media-store-01')
    await expect(rows.filter({ hasText: 'from its repository host' }).first()).toBeVisible()
  })

  test('names a schedule without a name by its number', async ({ page }) => {
    await loginAsAdmin(page)
    // The demo names every schedule, so the usage list is answered here.
    const id = await seededIdByName(page, '/api/dependency-hosts', 'nas-media')
    await page.route(`**/api/dependency-hosts/${id}/usage`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            schedule_id: 42,
            schedule_name: '',
            agent_id: 1,
            hostname: 'web-server-01',
            source: 'schedule',
          },
        ]),
      }),
    )
    await page.goto(`/dependency-hosts/${id}?section=used-by`)

    const row = page.locator('a.agent-row')
    await expect(row.locator('.agent-row-name')).toHaveText('Schedule #42')
    await expect(row).toHaveAttribute('href', '/schedules/42')
  })
})
