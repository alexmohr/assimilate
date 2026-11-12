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
  await page.locator('.list-row', { hasText: SCHEDULE_NAME }).first().click()
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

    const strips = page.locator('.runs-row')
    await expect(strips).toHaveCount(2)
    await expect(strips.nth(0).locator('.runs-row-name')).toHaveText('server-daily')
    await expect(strips.nth(1).locator('.runs-row-name')).toHaveText('media-weekly')
  })

  // The seeded reports carry no run id, so each copy is a run of its own: the
  // newest - the failed best-effort copy - is picked first, and the other
  // repository's strip picks its own.
  test('the run detail says which repository each run wrote into', async ({ page }) => {
    await openDualTargetSchedule(page)

    // Each host row of the run names the repository under the host.
    const pills = page.locator('.run-detail .log-row-subtitle')
    await expect(pills).toHaveText(['media-weekly'])

    await page.locator('.runs-row').nth(0).locator('.run-pill').last().click()
    await expect(pills).toHaveText(['server-daily'])
  })

  test('the backups tab lists both repositories, each archive saying which', async ({ page }) => {
    await openDualTargetSchedule(page)
    await page.getByRole('tab', { name: 'Backups' }).click()

    // Named in write order, each with the archives it holds.
    const filter = page.locator('.archives-panel').getByRole('group', { name: 'Repository' })
    await expect(filter).toBeVisible({ timeout: 15_000 })
    const labels = await filter.getByRole('button').allTextContents()
    expect(labels).toHaveLength(3)
    expect(labels[0]).toContain('All')
    expect(labels[1]).toContain('server-daily')
    expect(labels[2]).toContain('media-weekly')

    // Each copy opens against its own repository: the archive header says
    // which one the file tree - and the Delete in its menu - acts on.
    const repoLink = page.locator('.archive-head-facts a[href^="/repos/"]')
    const rows = page.locator('.archives-panel').getByTestId('archive-row')
    await filter.getByRole('button', { name: /server-daily/ }).click()
    await rows.first().click()
    await expect(repoLink).toHaveText('server-daily')

    // Under All, each row names its repository.
    await filter.getByRole('button', { name: /^All/ }).click()
    await expect(rows.filter({ hasText: 'media-weekly' }).first()).toBeVisible()

    await filter.getByRole('button', { name: /media-weekly/ }).click()
    const weeklyRow = rows.first()
    await weeklyRow.click()
    await expect(repoLink).toHaveText('media-weekly')
  })

  test('a single-repository schedule keeps its pane unscoped', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Backups' }).click()

    await expect(page.getByTestId('archive-row').first()).toBeVisible({ timeout: 15_000 })
    await expect(
      page.locator('.archives-panel').getByRole('group', { name: 'Repository' }),
    ).toHaveCount(0)
    await page.getByRole('tab', { name: 'Overview' }).click()
    // One strip of runs, under the one repository's name.
    await expect(page.locator('.runs-row')).toHaveCount(1)
  })
})
