// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A schedule that copies into more than one repository, end to end.
 *
 * The demo's "Web server dual-target" schedule writes into server-daily
 * (required) and media-weekly (best effort), and its seeded history has the
 * required copy landing while the best-effort one failed on the most recent
 * occurrence - so every screen below has one healthy repository and one
 * broken one to tell apart. See seed-demo.sh.
 */
const SCHEDULE_NAME = 'Web server dual-target'

async function openDualTargetSchedule(page: Page): Promise<void> {
  await loginAsAdmin(page)
  await page.goto('/schedules')
  await page.waitForLoadState('networkidle')
  await page.locator('.entity-card', { hasText: SCHEDULE_NAME }).first().click()
  await expect(page.getByRole('heading', { name: SCHEDULE_NAME })).toBeVisible()
  await page.waitForLoadState('networkidle')
}

test.describe('Schedule with several target repositories', () => {
  test('the overview reports each repository outcome separately', async ({ page }) => {
    await openDualTargetSchedule(page)

    const repoRows = page.locator('.repo-run')
    await expect(repoRows).toHaveCount(2)

    const required = repoRows.filter({ hasText: 'server-daily' })
    await expect(required.locator('.badge--success')).toBeVisible()
    await expect(required).not.toContainText('best effort')

    // The best-effort target's failure is what "On failure" does not stop,
    // so it has to be readable as a failure without reading as the run's.
    const bestEffort = repoRows.filter({ hasText: 'media-weekly' })
    await expect(bestEffort).toContainText('best effort')
    await expect(bestEffort.locator('.badge--danger')).toBeVisible()
  })

  test('recent runs are drawn one strip per repository', async ({ page }) => {
    await openDualTargetSchedule(page)

    const strips = page.locator('.repo-strip')
    await expect(strips).toHaveCount(2)
    await expect(strips.nth(0).locator('.group-label')).toHaveText('server-daily')
    await expect(strips.nth(1).locator('.group-label')).toHaveText('media-weekly')
  })

  test('recent backups say which repository each run wrote into', async ({ page }) => {
    await openDualTargetSchedule(page)

    const pills = page.locator('.agent-row .meta-pill')
    await expect(pills.filter({ hasText: 'server-daily' }).first()).toBeVisible()
    await expect(pills.filter({ hasText: 'media-weekly' }).first()).toBeVisible()
  })

  test('the backups tab scopes browsing to the chosen repository', async ({ page }) => {
    await openDualTargetSchedule(page)
    await page.getByRole('tab', { name: 'Backups' }).click()

    const scope = page.locator('#schedule-repo-scope')
    await expect(scope).toBeVisible()

    // Named in write order, each with the archives it holds.
    const labels = await scope.locator('option').allTextContents()
    expect(labels).toHaveLength(2)
    expect(labels[0]).toContain('server-daily')
    expect(labels[1]).toContain('media-weekly')

    // It opens on the schedule's primary target, and the archive header says
    // which repository the file tree - and the Delete beside it - acts on.
    const firstRow = page.locator('.archive-row').first()
    await expect(firstRow).toBeVisible({ timeout: 15_000 })
    await firstRow.click()
    await expect(page.locator('.archive-meta-bar .repo-link')).toHaveText('server-daily')

    // Switching re-scopes the whole pane. The selection goes with it: it named
    // an archive in the repository being left behind.
    await scope.selectOption({ index: 1 })
    await expect(page.locator('.archive-meta-bar')).toHaveCount(0)

    const weeklyRow = page.locator('.archive-row').first()
    await expect(weeklyRow).toBeVisible({ timeout: 15_000 })
    await weeklyRow.click()
    await expect(page.locator('.archive-meta-bar .repo-link')).toHaveText('media-weekly')
  })

  test('a single-repository schedule keeps its pane unscoped', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Backups' }).click()

    await expect(page.locator('#schedule-repo-scope')).toHaveCount(0)
    await expect(page.locator('.repo-strip')).toHaveCount(0)
  })
})
