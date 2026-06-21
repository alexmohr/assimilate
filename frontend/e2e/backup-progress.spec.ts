// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page, WebSocketRoute } from '@playwright/test'

const SCHEDULE_ID = 42
const REPO_ID = 7
const REPO_NAME = 'server-daily'

async function mockScheduleDetailApis(page: Page): Promise<void> {
  await page.route('**/api/agents', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([{ id: 1, hostname: 'web-server-01', display_name: null }]),
    }),
  )
  await page.route('**/api/repos', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([{ id: REPO_ID, name: REPO_NAME, repo_path: 'ssh://backup@host/repo' }]),
    }),
  )
  await page.route(`**/api/schedules/${SCHEDULE_ID}/reports*`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        {
          id: 1,
          agent_id: 1,
          repo_id: REPO_ID,
          schedule_id: SCHEDULE_ID,
          status: 'success',
          started_at: new Date(Date.now() - 3_600_000).toISOString(),
          finished_at: new Date(Date.now() - 3_600_000 + 300_000).toISOString(),
          original_size: 4_294_967_296,
          compressed_size: 2_147_483_648,
          deduplicated_size: 1_073_741_824,
          files_processed: 50_000,
          duration_secs: 300,
          error_message: null,
          warnings: [],
          borg_version: null,
          archive_name: 'web-server-01-backup-2026-01-01',
          run_id: null,
        },
      ]),
    }),
  )
  await page.route(`**/api/schedules/${SCHEDULE_ID}/targets*`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([{ agent_id: 1, execution_order: 0 }]),
    }),
  )
  await page.route(`**/api/schedules/${SCHEDULE_ID}/sources*`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        backup_sources: ['/home'],
        backup_sources_per_agent: null,
        exclude_patterns_per_agent: null,
      }),
    }),
  )
  await page.route(`**/api/schedules/${SCHEDULE_ID}`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        id: SCHEDULE_ID,
        repo_id: REPO_ID,
        name: 'Daily Backup',
        schedule_type: 'backup',
        cron_expression: '0 2 * * *',
        enabled: true,
        canary_enabled: false,
        last_run_at: null,
        next_run_at: null,
        exclude_patterns_raw: '',
        ignore_global_excludes: false,
        keep_hourly: 24,
        keep_daily: 7,
        keep_weekly: 4,
        keep_monthly: 12,
        keep_yearly: 10,
        compact_enabled: false,
        pre_backup_commands: '',
        post_backup_commands: '',
        on_failure: 'stop',
      }),
    }),
  )
}

function sendWsMsg(ws: WebSocketRoute, type: string, payload: unknown): void {
  ws.send(JSON.stringify({ type, payload }))
}

function makeArchiveProgressLine(nfiles: number, originalSize: number, path: string): string {
  return JSON.stringify({ type: 'archive_progress', nfiles, original_size: originalSize, path })
}

test.describe('backup progress card', () => {
  let ws: WebSocketRoute | null = null

  test.beforeEach(async ({ page }) => {
    ws = null
    let resolveWs!: (w: WebSocketRoute) => void
    const wsReady = new Promise<WebSocketRoute>((resolve) => {
      resolveWs = resolve
    })

    await page.routeWebSocket('**/ws/ui', (route) => {
      ws = route
      resolveWs(route)
    })

    await mockScheduleDetailApis(page)
    await loginAsAdmin(page)
    ws = await wsReady

    await page.goto(`/schedules/${SCHEDULE_ID}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
  })

  test('card appears when BackupStarted arrives', async ({ page }) => {
    await expect(page.locator('.live-log-card')).not.toBeVisible()

    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })

    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.live-log-title')).toContainText('Backup in progress')
    await expect(page.locator('.live-log-host-badge')).toContainText('web-server-01')
    await expect(page.locator('.live-log-empty')).toBeVisible()
  })

  test('progress data updates when BackupLog with archive_progress arrives', async ({ page }) => {
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })

    sendWsMsg(ws!, 'BackupLog', {
      hostname: 'web-server-01',
      schedule_id: SCHEDULE_ID,
      repo_id: REPO_ID,
      line: makeArchiveProgressLine(24_567, 2_147_483_648, '/home/user/documents/report.pdf'),
    })

    await expect(page.locator('.live-log-empty')).not.toBeVisible({ timeout: 3_000 })
    await expect(page.locator('.progress-body')).toContainText('24,567')
    await expect(page.locator('.progress-body')).toContainText('2.0 GB')
    await expect(page.locator('.progress-body')).toContainText('report.pdf')
  })

  test('estimated remaining appears when reference report exists', async ({ page }) => {
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })

    // Let the elapsed timer fire at least once (1 s) before sending progress data.
    await page.waitForTimeout(1_100)

    // 2 GB of 4 GB ≈ 50% done → estimated remaining ≈ elapsed (non-zero).
    sendWsMsg(ws!, 'BackupLog', {
      hostname: 'web-server-01',
      schedule_id: SCHEDULE_ID,
      repo_id: REPO_ID,
      line: makeArchiveProgressLine(25_000, 2_147_483_648, '/home/user/data.db'),
    })

    await expect(page.locator('.progress-body')).toContainText('Est. remaining', { timeout: 3_000 })
    await expect(page.locator('.progress-body')).toContainText('Elapsed')
  })

  test('card hides when BackupCompleted arrives', async ({ page }) => {
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })

    sendWsMsg(ws!, 'BackupCompleted', { hostname: 'web-server-01', target_name: REPO_NAME })

    await expect(page.locator('.live-log-card')).not.toBeVisible({ timeout: 5_000 })
  })

  test('BackupLog for a different repo is ignored', async ({ page }) => {
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.live-log-empty')).toBeVisible()

    sendWsMsg(ws!, 'BackupLog', {
      hostname: 'web-server-01',
      schedule_id: SCHEDULE_ID,
      repo_id: REPO_ID + 999,
      line: makeArchiveProgressLine(99, 1_000, '/other/file'),
    })

    // Progress placeholder should still be visible — no data arrived for our repo.
    await page.waitForTimeout(300)
    await expect(page.locator('.live-log-empty')).toBeVisible()
  })

  test('BackupStarted for a different repo does not show card', async ({ page }) => {
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: 'media-weekly' })

    await page.waitForTimeout(500)
    await expect(page.locator('.live-log-card')).not.toBeVisible()
  })
})

// These tests cover the two bugs fixed in the backup-progress-ui-stuck regression:
//   1. borg create was missing --progress, so archive_progress lines were never emitted.
//   2. The server never broadcast ServerToUi::BackupStarted, so the hostname badge and
//      elapsed timer were never initialised when a backup started mid-session.
//
// Both scenarios are exercised by loading the page while a backup is already running
// (the UI sets backupRunning via the reports API) and then verifying that the subsequent
// BackupStarted WS message and a BackupLog archive_progress line produce the correct UI.
test.describe('backup progress card — mid-backup page load', () => {
  let ws: WebSocketRoute | null = null

  test.beforeEach(async ({ page }) => {
    ws = null
    let resolveWs!: (w: WebSocketRoute) => void
    const wsReady = new Promise<WebSocketRoute>((resolve) => {
      resolveWs = resolve
    })

    await page.routeWebSocket('**/ws/ui', (route) => {
      ws = route
      resolveWs(route)
    })

    // Override the reports endpoint to return an in-progress report so that
    // loadReports() sets backupRunning = true before any WS message arrives.
    await mockScheduleDetailApis(page)
    await page.route(`**/api/schedules/${SCHEDULE_ID}/reports*`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            id: 2,
            agent_id: 1,
            repo_id: REPO_ID,
            schedule_id: SCHEDULE_ID,
            status: 'started',
            started_at: new Date(Date.now() - 30_000).toISOString(),
            finished_at: null,
            original_size: null,
            compressed_size: null,
            deduplicated_size: null,
            files_processed: null,
            duration_secs: null,
            error_message: null,
            warnings: [],
            borg_version: null,
            archive_name: null,
            run_id: null,
          },
        ]),
      }),
    )

    await loginAsAdmin(page)
    ws = await wsReady

    await page.goto(`/schedules/${SCHEDULE_ID}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
  })

  test('card is shown from DB report state without a WS BackupStarted', async ({ page }) => {
    // The reports API returned a 'started' report; the card must be visible before
    // any WS message is delivered.
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.live-log-title')).toContainText('Backup in progress')
    // Hostname badge is absent until BackupStarted arrives from the server.
    await expect(page.locator('.live-log-host-badge')).not.toBeVisible()
    await expect(page.locator('.live-log-empty')).toBeVisible()
  })

  test('hostname badge appears when BackupStarted arrives after mid-backup page load', async ({
    page,
  }) => {
    // Card is already visible from the DB report.
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.live-log-host-badge')).not.toBeVisible()

    // The server now broadcasts BackupStarted when it receives AgentToServer::BackupStarted.
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })

    await expect(page.locator('.live-log-host-badge')).toContainText('web-server-01', {
      timeout: 3_000,
    })
  })

  test('progress data appears when BackupLog arrives after mid-backup page load', async ({
    page,
  }) => {
    // Card is already visible from the DB report; BackupStarted initialises the session.
    await expect(page.locator('.live-log-card')).toBeVisible({ timeout: 5_000 })
    sendWsMsg(ws!, 'BackupStarted', { hostname: 'web-server-01', target_name: REPO_NAME })
    await expect(page.locator('.live-log-host-badge')).toBeVisible({ timeout: 3_000 })

    // A BackupLog line carrying an archive_progress JSON object (produced by
    // borg create --progress --log-json) must replace "Waiting for progress…".
    sendWsMsg(ws!, 'BackupLog', {
      hostname: 'web-server-01',
      schedule_id: SCHEDULE_ID,
      repo_id: REPO_ID,
      line: makeArchiveProgressLine(12_345, 1_073_741_824, '/var/lib/database/data.db'),
    })

    await expect(page.locator('.live-log-empty')).not.toBeVisible({ timeout: 3_000 })
    await expect(page.locator('.progress-body')).toContainText('12,345')
    await expect(page.locator('.progress-body')).toContainText('1.0 GB')
    await expect(page.locator('.progress-body')).toContainText('data.db')
  })
})
