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
    error_message:
      'file changed while being read: /var/www/config.php; slow read on /var/log/nginx/access.log',
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

  const warningRow = page.locator('.run-card:not(.run-card-system)').filter({ hasText: 'warning' })
  await expect(warningRow.first()).toBeVisible({ timeout: 10_000 })

  await warningRow.first().locator('.run-card-summary').click()
  await page.waitForTimeout(500)

  const warningText = page.locator('.warning-pre')
  await expect(warningText).toBeVisible({ timeout: 10_000 })
  await expect(warningText).toContainText('file changed while being read')
  await expect(warningText).toContainText('slow read on /var/log/nginx/access.log')

  // A warning-only report must not also render a duplicate Error box.
  await expect(page.locator('.error-pre')).toHaveCount(0)
})

test('acknowledges a warning row and can undo it', async ({ page }: { page: Page }) => {
  await loginAsAdmin(page)

  await page.route('**/api/agents', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/schedules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/stats/system-events**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )

  // The view mutates the row in place on a successful ack/unack rather than
  // refetching the list, so the activity feed only needs to be served once.
  await page.route('**/api/stats/activity**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([{ ...makeActivityRow(), acknowledged: false }]),
    }),
  )
  let ackRequests = 0
  await page.route('**/api/stats/activity/9999/acknowledge', async (route) => {
    ackRequests += 1
    await route.fulfill({ status: 204, body: '' })
  })

  await page.goto('/activity')
  await page.waitForTimeout(1000)

  const warningRow = page.locator('.run-card:not(.run-card-system)').filter({ hasText: 'warning' })
  await expect(warningRow.first()).toBeVisible({ timeout: 10_000 })

  await warningRow.first().getByRole('button', { name: 'Acknowledge' }).click()
  await expect(warningRow.first().getByText('Acknowledged')).toBeVisible({ timeout: 10_000 })
  const unackButton = warningRow.first().getByRole('button', { name: 'Unacknowledge' })
  await expect(unackButton).toBeVisible()

  await unackButton.click()
  await expect(warningRow.first().getByText('Acknowledged')).not.toBeVisible({ timeout: 10_000 })
  await expect(warningRow.first().getByRole('button', { name: 'Acknowledge' })).toBeVisible()
  expect(ackRequests).toBe(2)
})
