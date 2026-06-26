// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

// Navigate to the first schedule card and return its numeric ID.
async function openFirstSchedule(page: Page): Promise<string> {
  await page.goto('/schedules')
  await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })
  await page.locator('.schedule-card').first().click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 10_000 })
  const match = page.url().match(/\/schedules\/(\d+)/)
  if (!match) throw new Error(`unexpected schedule URL: ${page.url()}`)
  return match[1]
}

// Create a schedule with a sleep pre-backup command so backups take long
// enough to observe in-progress UI state. The schedule is created via the
// browser UI as required. Returns the numeric schedule ID.
async function createSlowScheduleViaUI(page: Page): Promise<string> {
  await page.goto('/schedules/new')

  // Wait for the host multi-select to be ready (agents loaded).
  await page.locator('.multi-select-trigger').waitFor({ timeout: 15_000 })

  // Open the host dropdown and select Production Web Server (web-server-01).
  await page.locator('.multi-select-trigger').click()
  await page.locator('.multi-select-dropdown').waitFor({ timeout: 5_000 })
  await page
    .locator('.multi-select-item')
    .filter({ hasText: 'Production Web Server' })
    .locator('input[type="checkbox"]')
    .check()
  // Close the dropdown by clicking the trigger again.
  await page.locator('.multi-select-trigger').click()

  // Give the schedule a recognisable name.
  await page.locator('input[placeholder="e.g. Daily web server backup"]').fill('e2e-lifecycle-slow')

  // Select the server-daily repository (wait for options to populate).
  const repoSelect = page.locator('select').filter({
    has: page.locator('option').filter({ hasText: 'server-daily' }),
  })
  await expect(repoSelect).toBeVisible({ timeout: 10_000 })
  await repoSelect.selectOption({ label: 'server-daily' })

  // Set a minimal backup source so borg has something to process.
  await page.locator('textarea[placeholder="Directories to back up, one per line"]').fill('/tmp')

  // Pre-backup commands are on the Advanced tab — click it first.
  await page.getByRole('button', { name: 'Advanced' }).click()

  // Add a pre-backup sleep so the in-progress state is observable.
  const preCmdArea = page.locator('textarea[placeholder*="One command per line, e.g."]')
  await preCmdArea.waitFor({ timeout: 5_000 })
  await preCmdArea.fill('sleep 5')

  // Submit the form.
  await page.getByRole('button', { name: 'Create Schedule' }).click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 15_000 })

  const match = page.url().match(/\/schedules\/(\d+)/)
  if (!match) throw new Error('Failed to get schedule ID from URL after creation')
  return match[1]
}

// ── Cancel-flow tests ─────────────────────────────────────────────────────────
// These tests need a backup that stays in-progress long enough to interact
// with it.  They share a single slow schedule and must run serially.

let slowScheduleId: string | null = null

test.describe.serial('cancel flow', () => {
  test.beforeAll(async ({ browser }) => {
    const page = await browser.newPage()
    try {
      await loginAsAdmin(page)
      slowScheduleId = await createSlowScheduleViaUI(page)
    } finally {
      await page.close()
    }
  })

  test.afterAll(async ({ browser }) => {
    if (!slowScheduleId) return
    const page = await browser.newPage()
    try {
      await loginAsAdmin(page)
      await page.request.delete(`/api/schedules/${slowScheduleId}`)
    } finally {
      await page.close()
    }
  })

  test('cancel button is shown when a backup is in progress', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(`/schedules/${slowScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    await expect(page.getByRole('button', { name: 'Cancel Backup' })).toBeVisible({
      timeout: 15_000,
    })
    await expect(runNowBtn).not.toBeVisible()

    // Cancel to leave a clean state for the next test.
    await page.getByRole('button', { name: 'Cancel Backup' }).click()
    await expect(runNowBtn).toBeVisible({ timeout: 30_000 })
  })

  test('clicking cancel sends the request and shows a toast', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(`/schedules/${slowScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    const cancelBtn = page.getByRole('button', { name: 'Cancel Backup' })
    await expect(cancelBtn).toBeVisible({ timeout: 15_000 })
    await cancelBtn.click()

    await expect(page.getByText(/cancel request sent/i)).toBeVisible({ timeout: 5_000 })
    await expect(runNowBtn).toBeVisible({ timeout: 30_000 })
  })

  test('after cancel the Run Now button is restored on next report poll', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(`/schedules/${slowScheduleId}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    const cancelBtn = page.getByRole('button', { name: 'Cancel Backup' })
    await expect(cancelBtn).toBeVisible({ timeout: 15_000 })
    await cancelBtn.click()

    await expect(page.getByText(/cancel request sent/i)).toBeVisible({ timeout: 5_000 })
    await expect(runNowBtn).toBeVisible({ timeout: 30_000 })
    await expect(cancelBtn).not.toBeVisible()
  })
})

// ── Run-now flow tests ────────────────────────────────────────────────────────

test.describe('run now flow', () => {
  test('Run Now shows a success toast when the API accepts the request', async ({ page }) => {
    await loginAsAdmin(page)
    await openFirstSchedule(page)

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    await expect(page.getByText(/started\./i)).toBeVisible({ timeout: 5_000 })

    // Wait for the backup to finish so it doesn't interfere with other tests.
    await expect(runNowBtn).toBeVisible({ timeout: 120_000 })
  })

  test('Run Now triggers a backup that eventually completes', async ({ page }) => {
    await loginAsAdmin(page)
    await openFirstSchedule(page)

    const runNowBtn = page.getByRole('button', { name: 'Run Now' })
    await expect(runNowBtn).toBeVisible({ timeout: 10_000 })
    await runNowBtn.click()

    // Either the cancel button appears (backup in progress) or Run Now reappears
    // immediately (backup completed before we could observe the in-progress state).
    const cancelOrRun = page.getByRole('button', { name: /cancel backup|run now/i })
    await expect(cancelOrRun).toBeVisible({ timeout: 15_000 })

    // Wait until the backup is no longer in progress.
    await expect(page.getByRole('button', { name: 'Run Now' })).toBeVisible({ timeout: 120_000 })
    await expect(page.getByRole('button', { name: 'Cancel Backup' })).not.toBeVisible()
  })
})
