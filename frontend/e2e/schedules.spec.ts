// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockScheduleOneHealth, test } from './fixtures'

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
  })

  test('overdue schedule card shows an Overdue chip with a per-host detail tooltip', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    // The demo's seeded health data has no overdue hosts, so this reproduces
    // a host whose own last report is stale even though the schedule itself
    // looks on track, which is exactly what the Overdue chip's tooltip
    // exists to surface.
    await mockScheduleOneHealth(page, { is_overdue: true })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.schedule-card', { hasText: 'server-daily' })
    const overdueChip = card.locator('.entity-issue-chip.sev-warning')
    await expect(overdueChip).toBeVisible()
    await expect(overdueChip).toContainText('Overdue')

    await expect(overdueChip).toHaveAttribute(
      'title',
      /Production Web Server \(web-server-01\) — last backup:/,
    )
  })

  test('running schedule card shows a Running pill', async ({ page }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, { last_status: 'started' })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.schedule-card', { hasText: 'server-daily' })
    const runningPill = card.locator('.entity-running-pill')
    await expect(runningPill).toBeVisible()
    await expect(runningPill).toContainText('Running')

    const otherCard = page.locator('.schedule-card', { hasText: 'database-hourly' })
    await expect(otherCard.locator('.entity-running-pill')).not.toBeVisible()
  })

  test("clicking a schedule card's Overdue chip navigates to the schedule detail page", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, { is_overdue: true })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.schedule-card', { hasText: 'server-daily' })
    const overdueChip = card.locator('.entity-issue-chip.sev-warning')
    await expect(overdueChip).toBeVisible()

    await overdueChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/schedules\/1$/)
  })

  test("clicking a schedule card's Failed chip navigates to the filtered activity log", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, {
      last_status: 'failed',
      last_error_message: 'Simulated failure',
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.schedule-card', { hasText: 'server-daily' })
    const failedChip = card.locator('.entity-issue-chip.sev-danger')
    await expect(failedChip).toBeVisible()

    await failedChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1&status=failed/)
  })

  test("clicking a schedule card's Warning chip navigates to the filtered activity log", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, {
      last_status: 'warning',
      last_error_message: 'Simulated warning',
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.schedule-card', { hasText: 'server-daily' })
    const warningChip = card.locator('.entity-issue-chip', { hasText: 'Warning' })
    await expect(warningChip).toBeVisible()

    await warningChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1&status=warning/)
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

    await page.goto(`/schedules/${staleSchedule!.id}`)
    await page.waitForLoadState('networkidle')

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
