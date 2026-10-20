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

  // The seeded reports carry no run id, so each copy is a run of its own: the
  // newest - the failed best-effort copy - is picked first, and the other
  // repository's strip picks its own.
  test('the run detail says which repository each run wrote into', async ({ page }) => {
    await openDualTargetSchedule(page)

    const pills = page.locator('.run-detail .agent-row .meta-pill')
    await expect(pills).toHaveText(['media-weekly'])

    await page.locator('.repo-strip').nth(0).locator('.run-pill').last().click()
    await expect(pills).toHaveText(['server-daily'])
  })

  test('the backups tab lists both repositories, each archive saying which', async ({ page }) => {
    await openDualTargetSchedule(page)
    await page.getByRole('tab', { name: 'Backups' }).click()

    // Named in write order, each with the archives it holds.
    const filter = page.locator('.archive-repo-filter')
    await expect(filter).toBeVisible({ timeout: 15_000 })
    const labels = await filter.locator('.segmented-option').allTextContents()
    expect(labels).toHaveLength(3)
    expect(labels[0]).toContain('All')
    expect(labels[1]).toContain('server-daily')
    expect(labels[2]).toContain('media-weekly')

    // Each copy opens against its own repository: the archive header says
    // which one the file tree - and the Delete beside it - acts on.
    await filter.locator('.segmented-option', { hasText: 'server-daily' }).click()
    await page.locator('.archive-row').first().click()
    await expect(page.locator('.archive-meta-bar .repo-link')).toHaveText('server-daily')

    await filter.locator('.segmented-option', { hasText: 'media-weekly' }).click()
    const weeklyRow = page.locator('.archive-row').first()
    await expect(weeklyRow.locator('.archive-repo')).toHaveText('media-weekly')
    await weeklyRow.click()
    await expect(page.locator('.archive-meta-bar .repo-link')).toHaveText('media-weekly')
  })

  test('a single-repository schedule keeps its pane unscoped', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Backups' }).click()

    await expect(page.locator('.archive-repo-filter')).toHaveCount(0)
    await expect(page.locator('.repo-strip')).toHaveCount(0)
  })
})
