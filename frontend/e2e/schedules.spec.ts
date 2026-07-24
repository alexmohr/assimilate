// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

interface ScheduleListEntry {
  id: number
  name: string
  target_hostnames: string[]
}

test.describe('Schedules management', () => {
  test('schedules list shows heading and seeded schedules', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Schedules' })).toBeVisible()
    await expect(page.getByText('server-daily').first()).toBeVisible()
    await expect(page.getByText('database-hourly').first()).toBeVisible()
    await expect(page.getByText('media-weekly').first()).toBeVisible()
    await expect(page.getByText('web-server-01').first()).toBeVisible()
    await expect(page.getByText('db-server-01').first()).toBeVisible()
    await expect(page.getByText('media-store-01').first()).toBeVisible()
  })

  test('overdue schedule card shows an expandable per-host detail toggle', async ({ page }) => {
    await loginAsAdmin(page)

    // The demo's seeded health data has no overdue hosts, so intercept
    // /api/stats/health and mark the web-server-01 / server-daily entry
    // (schedule 1, see 'schedule detail shows cron expression...' below)
    // overdue without an error message - this reproduces a host whose own
    // last report is stale even though the schedule itself looks on track,
    // which is exactly what the expandable "N host(s) overdue" toggle exists
    // to surface.
    await page.route(
      (url) => url.pathname === '/api/stats/health',
      async (route) => {
        const response = await route.fetch()
        const entries = (await response.json()) as Array<Record<string, unknown>>
        const withoutTarget = entries.filter(
          (e) => !(e.schedule_id === 1 && e.hostname === 'web-server-01'),
        )
        withoutTarget.push({
          schedule_id: 1,
          hostname: 'web-server-01',
          target_name: 'server-daily',
          last_status: 'success',
          last_backup_at: '2020-01-01T02:00:00Z',
          is_overdue: true,
          last_error_message: null,
          cron_expression: '0 2 * * *',
          schedule_enabled: true,
        })
        return route.fulfill({
          status: response.status(),
          contentType: 'application/json',
          body: JSON.stringify(withoutTarget),
        })
      },
    )

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.schedule-card', { hasText: 'server-daily' })
    await expect(card.getByText('1 host overdue')).toBeVisible()
    await expect(
      card.getByText('Production Web Server (web-server-01) — last backup:'),
    ).not.toBeVisible()

    await card.getByText('1 host overdue').click()

    await expect(
      card.getByText('Production Web Server (web-server-01) — last backup:'),
    ).toBeVisible()
  })

  test('clicking a schedule navigates to detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await page.getByText('server-daily').first().click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/schedules\/\d+/)
  })

  test('schedule detail shows cron expression and human-readable description', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.cron-input')).toHaveValue('0 2 * * *')
    await expect(page.getByText('Daily at 02:00').first()).toBeVisible()
  })

  test('schedule detail shows retention policy', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Retention' })).toBeVisible()
    await expect(page.getByText('Daily', { exact: true })).toBeVisible()
    await expect(page.getByText('Weekly', { exact: true })).toBeVisible()
  })

  test('schedule detail shows host and repository assignment', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    const infoCard = page.locator('.info-card')
    await expect(infoCard.getByText('Targets', { exact: true })).toBeVisible()
    await expect(infoCard.getByText('Repository', { exact: true })).toBeVisible()
    await expect(page.getByText('server-daily').first()).toBeVisible()
  })

  test('schedule detail Logs link navigates to activity log filtered by schedule', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await page.getByRole('button', { name: /Logs/ }).click()
    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1/)
  })

  test('schedule detail with per-host backup sources loads without error', async ({ page }) => {
    await loginAsAdmin(page)

    // Find the multi-agent schedule seeded with backup_sources_per_agent.
    const listResp = await page.request.get('/api/schedules')
    expect(listResp.ok()).toBe(true)
    const schedules = (await listResp.json()) as ScheduleListEntry[]

    const multiHost = schedules.find(
      (s) =>
        s.target_hostnames.includes('web-server-01') &&
        s.target_hostnames.includes('db-server-01') &&
        s.target_hostnames.includes('media-store-01'),
    )
    expect(multiHost).toBeDefined()

    // Navigate to the detail page - this used to crash before the null-safety fix.
    await page.goto(`/schedules/${multiHost!.id}`)
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(`/schedules/${multiHost!.id}`)

    // Per-host backup sources section should be rendered.
    await expect(
      page.locator('.per-host-paths').or(page.locator('.per-host-entry')).first(),
    ).toBeVisible()
  })

  test('schedule detail shows a Retry button for an overdue target and re-runs just that host', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    // stale-report-01 is seeded with a backdated backup report, so its
    // schedule always shows this target as overdue - see seed-demo.sh.
    const listResp = await page.request.get('/api/schedules')
    expect(listResp.ok()).toBe(true)
    const schedules = (await listResp.json()) as ScheduleListEntry[]
    const staleSchedule = schedules.find((s) => s.name === 'Stale nightly report')
    expect(staleSchedule).toBeDefined()

    const [healthResponse] = await Promise.all([
      page.waitForResponse((resp) => resp.url().includes('/api/stats/health') && resp.ok()),
      page.goto(`/schedules/${staleSchedule!.id}`),
    ])
    expect(healthResponse.ok()).toBe(true)
    await page.waitForLoadState('networkidle')

    // Diagnostic: assert directly on the API payload the page itself received,
    // so a failure here pinpoints a backend/seed data problem instead of only
    // showing "Overdue text never appeared" with no indication of why.
    const healthEntries = (await healthResponse.json()) as Array<{
      schedule_id: number
      hostname: string
      is_overdue: boolean
      last_backup_at: string | null
      last_status: string | null
      cron_expression: string | null
    }>
    const staleHealthEntry = healthEntries.find(
      (h) => h.schedule_id === staleSchedule!.id && h.hostname === 'stale-report-01',
    )
    expect(
      staleHealthEntry,
      `no health entry for stale-report-01 in: ${JSON.stringify(healthEntries)}`,
    ).toBeDefined()
    expect(
      staleHealthEntry?.is_overdue,
      `health entry was not overdue: ${JSON.stringify(staleHealthEntry)}`,
    ).toBe(true)

    const targetsRow = page.locator('.info-row-targets')
    await expect(targetsRow.getByText('Overdue')).toBeVisible({ timeout: 10_000 })
    const retryButton = targetsRow.getByRole('button', { name: 'Retry' })
    await expect(retryButton).toBeVisible()

    const [runResponse] = await Promise.all([
      page.waitForResponse(
        (resp) =>
          /\/api\/schedules\/\d+\/run$/.test(resp.url()) && resp.request().method() === 'POST',
      ),
      retryButton.click(),
    ])
    expect(runResponse.ok()).toBe(true)
    expect(runResponse.request().postDataJSON()).toEqual({
      agent_ids: [expect.any(Number)],
    })
  })

  test('creating a new schedule succeeds (regression: agent_ids/_per_agent field naming)', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/new')
    await page.waitForLoadState('networkidle')

    const targetCard = page.locator('.form-card', { hasText: 'Target' })

    await targetCard.locator('.multi-select-trigger').click()
    await targetCard.getByText('Production Web Server').click()

    // Close the dropdown so it doesn't cover the repository select.
    await page.getByPlaceholder('e.g. Daily web server backup').click()

    await targetCard
      .locator('.form-group', { hasText: 'Repository' })
      .locator('select')
      .selectOption({ label: 'server-daily' })

    // Use Integrity Check so the test doesn't depend on backup source paths.
    await targetCard
      .locator('.form-group', { hasText: 'Schedule Type' })
      .locator('select')
      .selectOption({ label: 'Integrity Check' })

    await page.getByRole('button', { name: 'Create Schedule' }).click()

    // The create request used to fail with "missing field `agent_ids`" because the
    // frontend sent client_ids/backup_sources_per_host instead of the names the
    // backend expects. A successful save navigates to the new schedule's detail page.
    await expect(page).toHaveURL(/\/schedules\/\d+$/)
    await expect(page.locator('.error-inline')).not.toBeVisible()
  })
})
