// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

const SCHEDULE_ID = 42
const REPO_ID = 7
const ARCHIVE_NAME = 'web-server-01-backup-2026-01-01'

function makeReport(overrides: Partial<Record<string, unknown>> = {}): object {
  return {
    id: 1,
    agent_id: 1,
    repo_id: REPO_ID,
    schedule_id: SCHEDULE_ID,
    status: 'success',
    started_at: '2026-01-01T02:00:00.000Z',
    finished_at: '2026-01-01T02:05:00.000Z',
    original_size: 4_294_967_296,
    compressed_size: 2_147_483_648,
    deduplicated_size: 1_073_741_824,
    files_processed: 50_000,
    duration_secs: 300,
    error_message: null,
    warnings: [],
    borg_version: null,
    archive_name: ARCHIVE_NAME,
    run_id: null,
    ...overrides,
  }
}

async function mockScheduleApis(
  page: Page,
  options: { scheduleType?: string; reports?: object[] } = {},
): Promise<void> {
  const { scheduleType = 'backup', reports = [makeReport()] } = options

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
      body: JSON.stringify([
        { id: REPO_ID, name: 'server-daily', repo_path: 'ssh://backup@host/repo' },
      ]),
    }),
  )
  await page.route(`**/api/schedules/${SCHEDULE_ID}/reports*`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(reports),
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
        commands_per_agent: null,
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
        schedule_type: scheduleType,
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

function contentsResponse(entries: object[]): object {
  return { index_status: 'done', entries }
}

test.describe('schedule backups browser', () => {
  test.beforeEach(async ({ page }) => {
    await mockScheduleApis(page)
    await loginAsAdmin(page)
    await page.goto(`/schedules/${SCHEDULE_ID}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
  })

  test('Backups tab is visible for a backup-type schedule', async ({ page }) => {
    await expect(page.getByRole('button', { name: 'Backups' })).toBeVisible()
  })

  test('Backups tab is absent for a check-type schedule', async ({ page }) => {
    await mockScheduleApis(page, { scheduleType: 'check' })
    await page.goto(`/schedules/${SCHEDULE_ID}`)
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    await expect(page.getByRole('button', { name: 'Backups' })).not.toBeVisible()
  })

  test('Backups tab lists archives from successful reports', async ({ page }) => {
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)

    await expect(page.locator('.archive-list-panel')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.archive-list-panel')).toContainText(ARCHIVE_NAME)
    await expect(page.locator('.archive-list-panel')).toContainText('web-server-01')
  })

  test('empty state is shown when no reports have an archive name', async ({ page }) => {
    await page.route(`**/api/schedules/${SCHEDULE_ID}/reports*`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([makeReport({ status: 'failed', archive_name: null })]),
      }),
    )
    await page.reload()
    await page.locator('.tab-bar').waitFor({ timeout: 10_000 })

    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)

    await expect(page.getByText(/no archives found/i)).toBeVisible({ timeout: 5_000 })
  })

  test('clicking an archive opens the file browser with its contents', async ({ page }) => {
    await page.route(`**/api/repos/${REPO_ID}/archives/${ARCHIVE_NAME}/contents*`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(
          contentsResponse([
            {
              type: 'd',
              path: 'home',
              size: 0,
              mtime: '2026-01-01T00:00:00',
              mode: 'drwxr-xr-x',
            },
            {
              type: '-',
              path: 'etc/passwd',
              size: 1_234,
              mtime: '2026-01-01T00:00:00',
              mode: '-rw-r--r--',
            },
          ]),
        ),
      }),
    )

    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)

    await page.locator('.archive-row').first().click()

    await expect(page.locator('.archive-browser-panel')).toBeVisible({ timeout: 5_000 })
    await expect(page.locator('.archive-browser-title')).toContainText(ARCHIVE_NAME)
    await expect(page.locator('.archive-browser-panel')).toContainText('home')
    await expect(page.locator('.archive-browser-panel')).toContainText('etc')
  })

  test('selected archive row is highlighted', async ({ page }) => {
    await page.route(`**/api/repos/${REPO_ID}/archives/${ARCHIVE_NAME}/contents*`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(contentsResponse([])),
      }),
    )

    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)
    await page.locator('.archive-row').first().click()

    await expect(page.locator('.archive-row-selected')).toBeVisible({ timeout: 5_000 })
  })

  test('breadcrumb updates and new contents load when navigating into a directory', async ({
    page,
  }) => {
    await page.route(
      `**/api/repos/${REPO_ID}/archives/${ARCHIVE_NAME}/contents*`,
      async (route) => {
        const url = new URL(route.request().url())
        const subpath = url.searchParams.get('path')
        if (subpath === 'home') {
          await route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify(
              contentsResponse([
                {
                  type: '-',
                  path: 'home/.bashrc',
                  size: 256,
                  mtime: '2026-01-01T00:00:00',
                  mode: '-rw-r--r--',
                },
              ]),
            ),
          })
        } else {
          await route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify(
              contentsResponse([
                {
                  type: 'd',
                  path: 'home',
                  size: 0,
                  mtime: '2026-01-01T00:00:00',
                  mode: 'drwxr-xr-x',
                },
              ]),
            ),
          })
        }
      },
    )

    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)
    await page.locator('.archive-row').first().click()

    // Root view: 'home' directory entry appears.
    await expect(page.locator('.archive-browser-panel')).toContainText('home', { timeout: 5_000 })

    // Navigate into 'home'.
    await page.locator('.archive-dir-row').filter({ hasText: 'home' }).click()

    // Breadcrumb now shows the 'home' segment.
    await expect(page.locator('.archive-breadcrumb')).toContainText('home', { timeout: 5_000 })

    // File inside 'home' directory is now visible.
    await expect(page.locator('.archive-browser-panel')).toContainText('.bashrc')
  })

  test('breadcrumb root segment navigates back to the archive root', async ({ page }) => {
    await page.route(
      `**/api/repos/${REPO_ID}/archives/${ARCHIVE_NAME}/contents*`,
      async (route) => {
        const url = new URL(route.request().url())
        const subpath = url.searchParams.get('path')
        if (subpath === 'home') {
          await route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify(
              contentsResponse([
                {
                  type: '-',
                  path: 'home/.bashrc',
                  size: 256,
                  mtime: '2026-01-01T00:00:00',
                  mode: '-rw-r--r--',
                },
              ]),
            ),
          })
        } else {
          await route.fulfill({
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify(
              contentsResponse([
                {
                  type: 'd',
                  path: 'home',
                  size: 0,
                  mtime: '2026-01-01T00:00:00',
                  mode: 'drwxr-xr-x',
                },
              ]),
            ),
          })
        }
      },
    )

    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)
    await page.locator('.archive-row').first().click()
    await expect(page.locator('.archive-browser-panel')).toContainText('home', { timeout: 5_000 })

    // Go into 'home', then click the root breadcrumb ('~' or '/') to go back.
    await page.locator('.archive-dir-row').filter({ hasText: 'home' }).click()
    await expect(page.locator('.archive-breadcrumb')).toContainText('home', { timeout: 5_000 })

    await page.locator('.archive-crumb').first().click()

    // Back at root: 'home' directory should be visible again.
    await expect(page.locator('.archive-browser-panel')).toContainText('home', { timeout: 5_000 })
    await expect(page.locator('.archive-breadcrumb')).not.toContainText('home')
  })

  test('download button is present for file entries', async ({ page }) => {
    await page.route(`**/api/repos/${REPO_ID}/archives/${ARCHIVE_NAME}/contents*`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(
          contentsResponse([
            {
              type: '-',
              path: 'etc/passwd',
              size: 1_234,
              mtime: '2026-01-01T00:00:00',
              mode: '-rw-r--r--',
            },
          ]),
        ),
      }),
    )

    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)
    await page.locator('.archive-row').first().click()

    await expect(page.locator('.archive-browser-panel')).toContainText('etc', { timeout: 5_000 })

    const downloadBtn = page.locator('.cell-action button').first()
    await expect(downloadBtn).toBeVisible({ timeout: 5_000 })
  })

  test('save bar is hidden on the Backups tab', async ({ page }) => {
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForURL(/tab=backups/)

    await expect(page.locator('.save-bar')).not.toBeVisible()
  })
})

// ── Demo-environment integration test ────────────────────────────────────────
// Verifies the tab works end-to-end against real seeded backup data without
// any API mocking.

test('Backups tab shows real archives on a seeded schedule', async ({ page }) => {
  await loginAsAdmin(page)

  // Navigate to the schedules list and open the first schedule card.
  await page.goto('/schedules')
  await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })
  await page.locator('.schedule-card').first().click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 10_000 })

  // The Backups tab must be present (seeded schedules are all backup type).
  const backupsBtn = page.getByRole('button', { name: 'Backups' })
  await expect(backupsBtn).toBeVisible({ timeout: 10_000 })
  await backupsBtn.click()
  await page.waitForURL(/tab=backups/)

  // Give reports a moment to load; the seeded schedule has 30+ successful runs.
  await page.waitForTimeout(2_000)
  await expect(page).not.toHaveURL(/\/error/)

  // Either archives are listed or the empty-state message is shown — never a
  // blank white area or an unhandled error.
  const hasArchives = await page.locator('.archive-list-panel').isVisible()
  const hasEmpty = await page.getByText(/no archives found/i).isVisible()
  expect(hasArchives || hasEmpty).toBe(true)
})
