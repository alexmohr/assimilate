// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockScheduleOneHealth, test } from './fixtures'
import type { Locator, Page } from '@playwright/test'

interface ScheduleListEntry {
  id: number
  name: string
  target_hostnames: string[]
}

// Navigates to the schedules list with schedule 1 ("server-daily") forced
// overdue, and returns its card and Overdue chip locators.
async function openOverdueScheduleCard(
  page: Page,
): Promise<{ card: Locator; overdueChip: Locator }> {
  await loginAsAdmin(page)
  await mockScheduleOneHealth(page, { is_overdue: true })

  await page.goto('/schedules')
  await page.waitForLoadState('networkidle')

  const card = page.locator('.entity-card', { hasText: 'server-daily' })
  const overdueChip = card.locator('.entity-issue-chip.sev-warning')
  return { card, overdueChip }
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

  test('schedules list groups cards under at least one time-based section and shows a run-history strip', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // Every schedule falls into exactly one time bucket (or Paused), so with
    // any seeded schedules present there is always at least one group header.
    await expect(page.locator('.schedule-group-header').first()).toBeVisible()

    const card = page.locator('.entity-card', { hasText: 'server-daily' }).first()
    await expect(card.locator('.run-history')).toBeVisible()
  })

  test('schedules list shows the 24h collision rail above the groups', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // The seeded schedules span hourly to daily cadences, so at least one
    // always has a next_run_at within the rail's 24h window.
    const rail = page.locator('.timeline-rail')
    await expect(rail).toBeVisible()
    await expect(rail.locator('.timeline-tick').first()).toBeVisible()
  })

  test('overdue schedule card shows an Overdue chip with a per-host detail tooltip', async ({
    page,
  }) => {
    // The demo's seeded health data has no overdue hosts, so this reproduces
    // a host whose own last report is stale even though the schedule itself
    // looks on track, which is exactly what the Overdue chip's tooltip
    // exists to surface.
    const { overdueChip } = await openOverdueScheduleCard(page)
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

    const card = page.locator('.entity-card', { hasText: 'server-daily' })
    const runningPill = card.locator('.entity-running-pill')
    await expect(runningPill).toBeVisible()
    await expect(runningPill).toContainText('Running')

    const otherCard = page.locator('.entity-card', { hasText: 'database-hourly' })
    await expect(otherCard.locator('.entity-running-pill')).not.toBeVisible()
  })

  test("clicking a schedule card's Overdue chip navigates to the schedule detail page", async ({
    page,
  }) => {
    const { overdueChip } = await openOverdueScheduleCard(page)
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

    const card = page.locator('.entity-card', { hasText: 'server-daily' })
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

    const card = page.locator('.entity-card', { hasText: 'server-daily' })
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

    // Overview's info summary already has the human-readable form; the
    // editable CronBuilder lives on Settings > General.
    await expect(page.getByText('Daily at 02:00').first()).toBeVisible()

    await page.getByRole('tab', { name: 'Settings' }).click()
    await expect(page.locator('.cron-input')).toHaveValue('0 2 * * *')
  })

  test('schedule detail shows retention policy', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.getByRole('button', { name: 'Retention' }).click()

    await expect(page.getByRole('heading', { name: 'Retention' })).toBeVisible()
    await expect(page.getByText('Daily', { exact: true })).toBeVisible()
    await expect(page.getByText('Weekly', { exact: true })).toBeVisible()
  })

  test('schedule detail shows host and repository assignment', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Overview is the default tab: the Targets section lists the schedule's
    // agents, and the info summary names the repository.
    await expect(page.getByRole('heading', { name: 'Targets' })).toBeVisible()
    const infoCard = page.locator('.panel', { hasText: 'Schedule info' })
    await expect(infoCard.getByText('Repository', { exact: true })).toBeVisible()
    await expect(page.getByText('server-daily').first()).toBeVisible()
  })

  test('schedule detail Logs link navigates to activity log filtered by schedule', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await page.getByRole('button', { name: 'More schedule actions' }).click()
    await page.getByRole('menuitem', { name: 'Logs' }).click()
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

    // Per-host backup sources live under Settings > Targets.
    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.getByRole('button', { name: 'Targets' }).click()
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

    // Overview is the default tab: an overdue target shows up both in the
    // attention banner and as a row in the Targets section, each with Retry.
    const attention = page.locator('.attention')
    await expect(attention).toBeVisible({ timeout: 15_000 })
    await expect(attention.getByText('Overdue')).toBeVisible({ timeout: 10_000 })
    const retryButton = attention.getByRole('button', { name: 'Retry' })
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

  test('toggles a schedule enabled/disabled from the list card without opening it', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    // Snapshot the seeded row so the mocked PUT response below can echo back a
    // complete ScheduleResponse, without ever sending the PUT to the real
    // backend (which would mutate demo data other, possibly-parallel, tests rely on).
    // Schedule 1 is the daily web-server-01/server-daily schedule (see other
    // tests in this file, e.g. the direct `/schedules/1` navigations below) -
    // its own `name` field is blank, so the card falls back to the repo name.
    // The other schedules seeded/created against the same repo across the
    // full e2e run also fall back to "server-daily", so the card is located
    // by its data-schedule-id rather than by text.
    const scheduleId = 1
    const getResp = await page.request.get(`/api/schedules/${scheduleId}`)
    expect(getResp.ok()).toBe(true)
    const original = (await getResp.json()) as Record<string, unknown>

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = page.locator(`.entity-card[data-schedule-id="${scheduleId}"]`)
    await expect(card.locator('.schedule-toggle-label')).toHaveText('Enabled')
    await expect(card).not.toHaveClass(/entity-card--notable/)

    await page.route(`**/api/schedules/${scheduleId}`, async (route) => {
      if (route.request().method() !== 'PUT') return route.fallback()
      const body = (await route.request().postDataJSON()) as Record<string, unknown>
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ...original, ...body }),
      })
    })

    const [putResponse] = await Promise.all([
      page.waitForResponse(
        (resp) =>
          resp.url().endsWith(`/api/schedules/${scheduleId}`) && resp.request().method() === 'PUT',
      ),
      card.locator('button[role="switch"]').click(),
    ])
    expect(putResponse.ok()).toBe(true)
    expect(putResponse.request().postDataJSON()).toMatchObject({ enabled: false })

    await expect(card.locator('.schedule-toggle-label')).toHaveText('Disabled')
    await expect(card).toHaveClass(/entity-card--notable/)
    await expect(card.locator('.entity-status-pill')).toHaveText('Disabled')
  })

  test('creating a new schedule succeeds (regression: agent_ids/_per_agent field naming)', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/new')
    await page.waitForLoadState('networkidle')

    // Hosts and Repository live under the Targets section; Schedule type
    // stays on General, where create mode opens.
    await page.getByRole('button', { name: 'Targets' }).click()
    const targetsSection = page.locator('.settings-pane')

    await targetsSection.locator('.multi-select-trigger').click()
    await targetsSection.getByText('Production Web Server').click()
    // Close the dropdown so it doesn't cover the repository select.
    await targetsSection.locator('.multi-select-trigger').click()

    await targetsSection
      .locator('.field', { hasText: 'Repository' })
      .locator('select')
      .selectOption({ label: 'server-daily' })

    // Use Integrity Check so the test doesn't depend on backup source paths.
    await page.getByRole('button', { name: 'General' }).click()
    await page
      .locator('.field', { hasText: 'Schedule type' })
      .locator('select')
      .selectOption({ label: 'Integrity check' })

    await page.getByRole('button', { name: 'Create schedule' }).click()

    // The create request used to fail with "missing field `agent_ids`" because the
    // frontend sent client_ids/backup_sources_per_host instead of the names the
    // backend expects. A successful save navigates to the new schedule's detail page.
    await expect(page).toHaveURL(/\/schedules\/\d+$/)
    await expect(page.locator('.error-inline')).not.toBeVisible()
  })
})
