// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/*
 * Retry on a failed run in the Activity log: it re-runs the run's schedule for
 * just that host and repository, and goes away once a newer run of that copy
 * exists. The feed, the host's report and the run request are mocked so the
 * test owns which run is failed and newest.
 */
test('retries a failed run from its detail in the Activity log', async ({ page }) => {
  await loginAsAdmin(page)

  const startedAt = new Date(Date.now() - 3_600_000).toISOString()
  const finishedAt = new Date(Date.now() - 3_540_000).toISOString()
  const failedEntry = {
    id: 9990,
    hostname: 'web-server-01',
    target_name: 'server-daily',
    started_at: startedAt,
    finished_at: finishedAt,
    status: 'failed',
    duration_secs: 60,
    repo_id: 1,
    archive_name: null,
    error_message: 'connection refused',
    schedule_id: 1,
    schedule_name: null,
    run_id: null,
    acknowledged: false,
  }
  const failedReport = {
    id: 9990,
    agent_id: 1,
    repo_id: 1,
    schedule_id: 1,
    started_at: startedAt,
    finished_at: finishedAt,
    status: 'failed',
    original_size: 0,
    compressed_size: 0,
    deduplicated_size: 0,
    files_processed: 0,
    duration_secs: 60,
    error_message: 'connection refused',
    warnings: [],
    borg_version: '1.4.1',
    archive_name: null,
    borg_command: null,
    hostname: 'web-server-01',
    repo_name: 'server-daily',
    schedule_name: null,
  }

  let retried = false
  await page.route(
    (url) => url.pathname === '/api/stats/activity',
    (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(
          retried
            ? [{ ...failedEntry, id: 9991, status: 'pending', started_at: finishedAt }, failedEntry]
            : [failedEntry],
        ),
      }),
  )
  await page.route('**/api/agents/web-server-01/reports**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ reports: [failedReport], total: 1 }),
    }),
  )
  let runBody: unknown = null
  await page.route('**/api/schedules/1/run', (route) => {
    runBody = route.request().postDataJSON()
    retried = true
    return route.fulfill({ status: 202, body: '' })
  })

  await page.goto('/activity?category=backup&report=9990')
  await page.waitForLoadState('networkidle')

  const retry = page.getByRole('button', { name: 'Retry' })
  await retry.click()

  await expect(page.getByText('Retry queued for server-daily on web-server-01.')).toBeVisible()
  expect(runBody).toEqual({ agent_ids: [1], repo_ids: [1] })
  // The queued run is now the newest of that copy, so the failure offers no retry.
  await expect(retry).toHaveCount(0)
})
