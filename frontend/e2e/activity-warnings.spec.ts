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

test('acknowledges a warning row, hides it, and can undo it via the filter', async ({
  page,
}: {
  page: Page
}) => {
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

  // The server hides acknowledged entries unless the caller asks for them, so
  // the mock has to honour the filter for the round trip to mean anything.
  let acknowledged = false
  await page.route('**/api/stats/activity?**', (route) => {
    const filter = new URL(route.request().url()).searchParams.get('acknowledged')
    const visible = filter === 'unacknowledged' ? !acknowledged : true
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(visible ? [{ ...makeActivityRow(), acknowledged }] : []),
    })
  })
  let ackRequests = 0
  await page.route('**/api/stats/activity/9999/acknowledge', async (route) => {
    ackRequests += 1
    acknowledged = route.request().method() === 'POST'
    await route.fulfill({ status: 204, body: '' })
  })

  await page.goto('/activity')
  await page.waitForTimeout(1000)

  const warningRow = page.locator('.run-card:not(.run-card-system)').filter({ hasText: 'warning' })
  await expect(warningRow.first()).toBeVisible({ timeout: 10_000 })

  await warningRow.first().getByRole('button', { name: 'Acknowledge' }).click()
  await expect(warningRow).toHaveCount(0, { timeout: 10_000 })

  const acknowledgedFilter = page
    .locator('select.select-input')
    .filter({ has: page.locator('option', { hasText: 'Only acknowledged' }) })
  await acknowledgedFilter.selectOption('all')
  await expect(warningRow.first()).toBeVisible({ timeout: 10_000 })
  await expect(warningRow.first().getByText('Acknowledged')).toBeVisible()

  const unackButton = warningRow.first().getByRole('button', { name: 'Unacknowledge' })
  await unackButton.click()
  await expect(warningRow.first().getByText('Acknowledged')).not.toBeVisible({ timeout: 10_000 })
  await expect(warningRow.first().getByRole('button', { name: 'Acknowledge' })).toBeVisible()
  expect(ackRequests).toBe(2)
})

test('acknowledges a failed periodic sync and clears it from the feed', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)

  await page.route('**/api/agents', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/schedules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/stats/activity**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )

  let acknowledged = false
  await page.route('**/api/stats/system-events**', (route) => {
    const filter = new URL(route.request().url()).searchParams.get('acknowledged')
    const visible = filter === 'unacknowledged' ? !acknowledged : true
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(
        visible
          ? [
              {
                id: 4242,
                created_at: new Date().toISOString(),
                event_type: 'repo_sync_failed',
                severity: 'failed',
                acknowledgeable: true,
                acknowledged,
                hostname: 'web-server-01',
                message: 'Repository sync failed: connection refused',
              },
            ]
          : [],
      ),
    })
  })
  await page.route('**/api/stats/system-events/4242/acknowledge', async (route) => {
    acknowledged = route.request().method() === 'POST'
    await route.fulfill({ status: 204, body: '' })
  })

  await page.goto('/activity')
  await page.waitForTimeout(1000)

  const syncEvent = page.locator('.run-card-system').filter({ hasText: 'Repository sync failed' })
  await expect(syncEvent.first()).toBeVisible({ timeout: 10_000 })

  await syncEvent.first().getByRole('button', { name: 'Acknowledge' }).click()
  await expect(syncEvent).toHaveCount(0, { timeout: 10_000 })
})

test('acknowledges everything outstanding in one click', async ({ page }: { page: Page }) => {
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

  let acknowledgedAll = false
  await page.route('**/api/stats/activity?**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(acknowledgedAll ? [] : [{ ...makeActivityRow(), acknowledged: false }]),
    }),
  )
  await page.route('**/api/stats/activity/acknowledge-all', async (route) => {
    acknowledgedAll = true
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ backup_reports: 1, system_events: 0 }),
    })
  })

  await page.goto('/activity')
  await page.waitForTimeout(1000)

  const warningRow = page.locator('.run-card:not(.run-card-system)').filter({ hasText: 'warning' })
  await expect(warningRow.first()).toBeVisible({ timeout: 10_000 })

  await page.getByRole('button', { name: 'Acknowledge all' }).click()
  await expect(warningRow).toHaveCount(0, { timeout: 10_000 })
  await expect(page.getByRole('button', { name: 'Acknowledge all' })).toHaveCount(0)
})
