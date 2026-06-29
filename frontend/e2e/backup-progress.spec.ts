// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

// A schedule with a sleep pre-backup command gives the tests a reliable
// window to observe in-progress UI state and mid-backup page loads.
// It is created once via the browser UI and deleted after the suite.
let progressScheduleId: string | null = null

// Create a slow test schedule for db-server-01 via the browser form so that
// backup-progress and backup-lifecycle tests can run in parallel without
// competing for the same agent.
async function createProgressScheduleViaUI(page: Page): Promise<string> {
  await page.goto('/schedules/new')

  await page.locator('.multi-select-trigger').waitFor({ timeout: 15_000 })

  await page.locator('.multi-select-trigger').click()
  await page.locator('.multi-select-dropdown').waitFor({ timeout: 5_000 })
  await page
    .locator('.multi-select-item')
    .filter({ hasText: 'Primary Database' })
    .locator('input[type="checkbox"]')
    .check()
  await page.locator('.multi-select-trigger').click()

  await page.locator('input[placeholder="e.g. Daily web server backup"]').fill('e2e-progress-slow')

  const repoSelect = page.locator('select').filter({
    has: page.locator('option').filter({ hasText: 'database-hourly' }),
  })
  await expect(repoSelect).toBeVisible({ timeout: 10_000 })
  await repoSelect.selectOption({ label: 'database-hourly' })

  await page.locator('textarea[placeholder="Directories to back up, one per line"]').fill('/tmp')

  // Pre-backup commands are on the Advanced tab — click it first.
  await page.getByRole('button', { name: 'Advanced' }).click()

  const preCmdArea = page.locator('textarea[placeholder*="One command per line, e.g."]')
  await preCmdArea.waitFor({ timeout: 5_000 })
  await preCmdArea.fill('sleep 5')

  await page.getByRole('button', { name: 'Create Schedule' }).click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 15_000 })

  const match = page.url().match(/\/schedules\/(\d+)/)
  if (!match) throw new Error('Failed to get schedule ID from URL after creation')
  return match[1]
}

// ── Backup progress card — schedule detail view ───────────────────────────────
// Tests run serially because they trigger backups on the same schedule.

test.describe.serial('backup progress card', () => {
  test.beforeAll(async ({ browser }) => {
    const page = await browser.newPage()
    try {
      await loginAsAdmin(page)
      progressScheduleId = await createProgressScheduleViaUI(page)
    } finally {
      await page.close()
    }
  })

  test('card appears when a backup is triggered', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    await expect(page.locator('.live-log-card')).not.toBeVisible()

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    // BackupStarted arrives quickly; the card should appear before the sleep ends.
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 15_000 })
    await expect(page.locator('.live-log-title')).toContainText('Backup in progress')

    // Wait for backup to finish so the next test starts clean.
    await expect(runNowBtn).toBeVisible({ timeout: 60_000 })
  })

  test('progress data appears during an active backup', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 15_000 })

    // archive_progress data arrives after the sleep finishes (~5 s); allow 30 s.
    await expect(page.locator('.progress-body')).toBeVisible({ timeout: 30_000 })

    await expect(runNowBtn).toBeVisible({ timeout: 60_000 })
  })

  test('card hides when backup completes', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 15_000 })

    // When BackupCompleted arrives the card must disappear.
    await expect(page.locator('.live-log-card')).not.toBeVisible({ timeout: 60_000 })
    await expect(runNowBtn).toBeVisible({ timeout: 5_000 })
  })
})

// ── Mid-backup page load ──────────────────────────────────────────────────────
// Verify that navigating to the schedule detail page while a backup is already
// running correctly shows the in-progress card from the DB report state.

test.describe.serial('backup progress card — mid-backup page load', () => {
  test.beforeAll(async ({ browser }) => {
    // Re-use the schedule created by the first describe block if it already
    // exists; otherwise create it now.
    if (progressScheduleId) return
    const page = await browser.newPage()
    try {
      await loginAsAdmin(page)
      progressScheduleId = await createProgressScheduleViaUI(page)
    } finally {
      await page.close()
    }
  })

  test('card and host badge are shown from DB report on page load mid-backup', async ({ page }) => {
    await loginAsAdmin(page)

    // Trigger a backup and immediately navigate away so the page has not seen
    // the BackupStarted WebSocket event yet.
    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    // Navigate away promptly — the sleep pre-backup gives us ~5 s before the
    // backup report transitions to complete.
    await page.goto('/schedules')
    await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })

    // Navigate back — the backup is still "started" in the DB.
    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    // The UI must show the in-progress card derived from the DB report.
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 10_000 })
    await expect(page.locator('.live-log-title')).toContainText('Backup in progress')

    // Wait for the backup to finish.
    await expect(runNowBtn).toBeVisible({ timeout: 60_000 })
  })

  test('progress data appears after mid-backup page load', async ({ page }) => {
    await loginAsAdmin(page)

    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    await page.goto('/schedules')
    await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })

    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 10_000 })

    // archive_progress BackupLog messages arrive after the pre-backup sleep.
    await expect(page.locator('.progress-body')).toBeVisible({ timeout: 30_000 })

    await expect(runNowBtn).toBeVisible({ timeout: 60_000 })
  })
})

// ── Activity log — live backup log ───────────────────────────────────────────
// Verify that the /activity page shows a live session card while a backup is
// running and that it disappears after completion.

test.describe.serial('activity log — live backup log', () => {
  test.beforeAll(async ({ browser }) => {
    if (progressScheduleId) return
    const page = await browser.newPage()
    try {
      await loginAsAdmin(page)
      progressScheduleId = await createProgressScheduleViaUI(page)
    } finally {
      await page.close()
    }
  })

  test.afterAll(async ({ browser }) => {
    if (!progressScheduleId) return
    const page = await browser.newPage()
    try {
      await loginAsAdmin(page)
      await page.request.delete(`/api/schedules/${progressScheduleId}`)
    } finally {
      await page.close()
    }
  })

  test('live session card appears when a backup is running', async ({ page }) => {
    await loginAsAdmin(page)

    // Start the backup from the schedule detail page, then quickly navigate to
    // /activity so we arrive while the backup is still in progress.
    await page.goto(`/schedules/${progressScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    await page.goto('/activity')
    await page.locator('.activity-log').waitFor({ timeout: 10_000 })

    // The live session card appears once borg emits its first log_message line
    // (after the pre-backup sleep). Allow up to 30 s for this.
    await expect(page.locator('.live-session-card')).toBeVisible({ timeout: 30_000 })

    // Wait for the backup to finish (card disappears).
    await expect(page.locator('.live-session-card')).not.toBeVisible({ timeout: 60_000 })
  })
})
