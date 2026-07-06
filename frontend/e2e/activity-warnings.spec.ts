// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

function makeWarningReport(): object {
  return {
    id: 9999,
    agent_id: 1,
    repo_id: 1,
    schedule_id: null,
    started_at: new Date(Date.now() - 3600_000).toISOString(),
    finished_at: new Date().toISOString(),
    status: 'warning',
    original_size: 1024,
    compressed_size: 512,
    deduplicated_size: 128,
    files_processed: 150,
    duration_secs: 300,
    error_message: null,
    warnings: [
      'file changed while being read: /var/www/config.php',
      'slow read on /var/log/nginx/access.log',
    ],
    borg_version: null,
    archive_name: null,
    borg_command: null,
    hostname: 'web-server-01',
    repo_name: null,
    schedule_name: null,
  }
}

function makeActivityRow(): object {
  return {
    id: 9999,
    hostname: 'web-server-01',
    target_name: 'server-daily',
    started_at: new Date(Date.now() - 3600_000).toISOString(),
    finished_at: new Date().toISOString(),
    status: 'warning',
    duration_secs: 300,
    schedule_id: null,
    schedule_name: null,
    run_id: null,
  }
}

test('expands warning report row and shows warning messages', async ({ page }: { page: Page }) => {
  await loginAsAdmin(page)

  await page.route('**/api/agents', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )

  await page.route('**/api/schedules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )

  await page.route('**/api/stats/activity**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([makeActivityRow()]),
    }),
  )

  await page.route('**/api/stats/system-events**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )

  const reportsUrl = '**/api/agents/web-server-01/reports**'
  await page.route(reportsUrl, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([makeWarningReport()]),
    }),
  )

  await page.goto('/activity')
  await page.waitForTimeout(1000)

  const warningRow = page.locator('tr.log-row').filter({ hasText: 'warning' })
  await expect(warningRow.first()).toBeVisible({ timeout: 10_000 })

  await warningRow.first().click()
  await page.waitForTimeout(500)

  const warningText = page.locator('.status-pre.warning-pre')
  await expect(warningText).toBeVisible({ timeout: 10_000 })
  await expect(warningText).toContainText('file changed while being read')
  await expect(warningText).toContainText('slow read on /var/log/nginx/access.log')
})
