// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

// Navigate to the first schedule in the list and return its numeric ID.
async function openFirstSchedule(page: Page): Promise<string> {
  await page.goto('/schedules')
  await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })
  await page.locator('.schedule-card').first().click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 10_000 })
  const match = page.url().match(/\/schedules\/(\d+)/)
  if (!match) throw new Error(`unexpected schedule URL: ${page.url()}`)
  return match[1]
}

// Minimal report row that satisfies the view's status checks.
function makeReport(status: 'started' | 'pending' | 'success' | 'cancelled'): object {
  return {
    id: 9999,
    agent_id: 1,
    repo_id: 1,
    schedule_id: 1,
    status,
    started_at: new Date().toISOString(),
    finished_at: new Date().toISOString(),
    original_size: 0,
    compressed_size: 0,
    deduplicated_size: 0,
    files_processed: 0,
    duration_secs: 0,
    error_message: null,
    warnings: [],
    matched: false,
    archive_name: null,
    run_id: 'test-run-id',
  }
}

// ── Cancel flow ──────────────────────────────────────────────────────────────

test('cancel button is shown when a backup is in progress', async ({ page }) => {
  await loginAsAdmin(page)
  const id = await openFirstSchedule(page)

  // Override the reports endpoint to report a running backup.
  await page.route(`**/api/schedules/${id}/reports**`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([makeReport('started')]),
    }),
  )
  await page.reload()
  await page.waitForTimeout(500)

  await expect(page.getByRole('button', { name: 'Cancel Backup' })).toBeVisible({ timeout: 10_000 })
  await expect(page.getByRole('button', { name: 'Run Now' })).not.toBeVisible()
})

test('clicking cancel sends the request and shows a toast', async ({ page }) => {
  await loginAsAdmin(page)
  const id = await openFirstSchedule(page)

  await page.route(`**/api/schedules/${id}/reports**`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([makeReport('started')]),
    }),
  )
  await page.reload()

  const cancelBtn = page.getByRole('button', { name: 'Cancel Backup' })
  await expect(cancelBtn).toBeVisible({ timeout: 10_000 })
  await cancelBtn.click()

  await expect(page.getByText(/cancel request sent/i)).toBeVisible({ timeout: 5_000 })
})

test('after cancel the Run Now button is restored on next report poll', async ({ page }) => {
  await loginAsAdmin(page)
  const id = await openFirstSchedule(page)

  // First call: backup is running. Subsequent calls: it has been cancelled.
  let callCount = 0
  await page.route(`**/api/schedules/${id}/reports**`, (route) => {
    callCount++
    const body = callCount === 1 ? [makeReport('started')] : [makeReport('cancelled')]
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) })
  })
  await page.reload()

  const cancelBtn = page.getByRole('button', { name: 'Cancel Backup' })
  await expect(cancelBtn).toBeVisible({ timeout: 10_000 })
  await cancelBtn.click()
  await expect(page.getByText(/cancel request sent/i)).toBeVisible({ timeout: 5_000 })

  // Navigate back to the same schedule — on reload the route mock returns
  // the cancelled report, so backupRunning becomes false.
  await page.goto(`/schedules/${id}`)
  await expect(page.getByRole('button', { name: 'Run Now' })).toBeVisible({ timeout: 10_000 })
  await expect(page.getByRole('button', { name: 'Cancel Backup' })).not.toBeVisible()
})

// ── Completion flow ──────────────────────────────────────────────────────────

test('Run Now triggers a backup that eventually completes', async ({ page }) => {
  await loginAsAdmin(page)
  await openFirstSchedule(page)

  // The demo runs real agents, so Run Now dispatches an actual borg operation.
  const runNowBtn = page.getByRole('button', { name: 'Run Now' })
  await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
  await runNowBtn.click()

  // Wait for the agent to pick up the job (Cancel Backup appears via BackupStarted WS event).
  // If the backup is so fast that it already completed, Run Now will still be visible — both
  // states are acceptable here; we just must not stay in an error state.
  const cancelOrRun = page.getByRole('button', { name: /cancel backup|run now/i })
  await expect(cancelOrRun).toBeVisible({ timeout: 15_000 })

  // Wait until the backup is no longer in progress (BackupCompleted resets the button).
  await expect(page.getByRole('button', { name: 'Run Now' })).toBeVisible({ timeout: 120_000 })
  await expect(page.getByRole('button', { name: 'Cancel Backup' })).not.toBeVisible()
})
