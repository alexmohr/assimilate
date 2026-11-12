// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import {
  expect,
  interceptScheduleSave,
  loginAsAdmin,
  scheduleIdByName,
  makeFailedReport,
  mockScheduleOneHealth,
  mockScheduleOnePatch,
  openSettingsSection,
  test,
} from './fixtures'
import type { Locator, Page } from '@playwright/test'

interface ScheduleListEntry {
  id: number
  name: string
  target_hostnames: string[]
}

/**
 * The Schedules list row for one seeded schedule, selected by its id.
 *
 * Not by its repository name: an unnamed schedule's row is titled after the
 * repository it backs up, and more than one seeded schedule backs up to
 * `server-daily` - so a repo-name filter matches several rows and every
 * assertion under it is one status chip away from a strict-mode violation.
 */
function scheduleRow(page: Page, id: number): Locator {
  return page.locator(`.list-row[data-schedule-id="${id}"]`)
}

/** A row's one health chip. */
function healthChip(row: Locator): Locator {
  return row.locator('.schedules-health')
}

/** Unfolds the Disabled group, which the list shows collapsed to its header. */
async function openDisabledGroup(page: Page): Promise<void> {
  await page.locator('.schedules-group-toggle').click()
}

// Fills a numeric settings field, intercepts the PUT so the response echoes
// back the value the request actually sent, and waits for the save to land.
// Returns the field's input so the caller can assert its final value.
/** One hook command as the schedule PUT carries it. */
interface HookCommandPayload {
  command: string
  timeout_seconds: number | null
}

/**
 * Opens schedule 1's Settings -> Advanced pane, where the hook command fields
 * live. Extracted rather than repeated per test: three tests need the same
 * five-step navigation, which the duplicate-code check counts as a cluster.
 */
async function openScheduleAdvanced(page: Page): Promise<void> {
  await loginAsAdmin(page)
  await page.goto('/schedules/1')
  await page.waitForLoadState('networkidle')

  await page.getByRole('tab', { name: 'Settings' }).click()
  await openSettingsSection(page, 'Advanced')
}

/** Adds an empty pre-backup command row, returning its field and its script box. */
async function addPreBackupCommand(page: Page): Promise<{ field: Locator; script: Locator }> {
  // The Advanced pane renders its settings as `.pane-row`s; the General
  // section still uses `.field` blocks.
  const field = page.locator('.pane-row', { hasText: 'Pre-backup commands' })
  await field.getByRole('button', { name: '+ Add command' }).click()
  return { field, script: field.locator('textarea').last() }
}

/** Saves the schedule and returns the pre-backup commands the UI sent. */
async function savePreBackupCommands(page: Page): Promise<HookCommandPayload[]> {
  const waitForSave = await interceptScheduleSave(page, 1, (requestBody, responseBody) => ({
    ...responseBody,
    pre_backup_commands: requestBody.pre_backup_commands,
  }))

  await page.getByRole('button', { name: 'Save changes' }).click()

  return (await waitForSave()).pre_backup_commands as HookCommandPayload[]
}

async function saveNumericScheduleField(
  page: Page,
  fieldLabel: string,
  jsonKey: string,
  newValue: number,
  inputSelector?: string,
): Promise<Locator> {
  // A plain fieldLabel match is ambiguous once a schedule has its own
  // pre/post-backup commands: their per-command timeout inputs sit inside
  // the same "Hook command timeout"-mentioning row (it names the default
  // they fall back to), so callers whose label is no longer unique on the
  // page pass the input's own selector instead.
  const input = inputSelector
    ? page.locator(inputSelector)
    : page.locator('.field, .pane-row', { hasText: fieldLabel }).locator('input[type="number"]')
  await expect(input).toBeVisible()

  const waitForSave = await interceptScheduleSave(page, 1, (requestBody, responseBody) => ({
    ...responseBody,
    [jsonKey]: requestBody[jsonKey],
  }))

  await input.fill(String(newValue))
  await page.getByRole('button', { name: 'Save changes' }).click()

  const savedBody = await waitForSave()
  expect(savedBody[jsonKey]).toBe(newValue)

  return input
}

// Navigates to the schedules list with schedule 1 forced overdue, and returns
// its card and Overdue chip locators.
async function openOverdueScheduleRow(page: Page): Promise<{ row: Locator; overdueChip: Locator }> {
  await loginAsAdmin(page)
  await mockScheduleOneHealth(page, { is_overdue: true })

  await page.goto('/schedules')
  await page.waitForLoadState('networkidle')

  const row = scheduleRow(page, 1)
  return { row, overdueChip: healthChip(row) }
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

  test('schedules list groups rows under at least one time-based section', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // Every schedule falls into exactly one time bucket (or Disabled), so with
    // any seeded schedules present there is always at least one group header.
    await expect(page.locator('.list-group-header').first()).toBeVisible()

    // A row is the status dot, the name over its cadence, and the next run.
    const row = page.locator('.list-row', { hasText: 'server-daily' }).first()
    await expect(row.locator('.status-dot')).toBeVisible()
    await expect(row.locator('.list-row-sub')).not.toBeEmpty()
  })

  test('the group control re-sections the list by agent and by repository', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // The one segmented control on the page; no option's label is a substring
    // of another, so a text filter picks exactly one.
    const groupOption = (label: string) =>
      page.locator('.segmented-option').filter({ hasText: label })
    const groupTitles = page.locator('.list-group-title')

    // The multi-host schedule seeded by seed-demo.sh targets all three demo
    // agents, so every one of them heads a section of its own.
    await groupOption('Agent').click()
    await expect(groupTitles.filter({ hasText: 'web-server-01' }).first()).toBeVisible()
    await expect(groupTitles.filter({ hasText: 'db-server-01' }).first()).toBeVisible()
    await expect(groupTitles.filter({ hasText: 'media-store-01' }).first()).toBeVisible()

    // Grouped by repository the sections are named after the repos instead,
    // and the weekly repo holds the schedule that writes into it.
    await groupOption('Repo').click()
    await expect(groupTitles.filter({ hasText: 'server-daily' }).first()).toBeVisible()
    const weekly = page
      .locator('.list-group')
      .filter({ has: page.locator('.list-group-title', { hasText: 'media-weekly' }) })
    await expect(weekly.locator('.list-row').first()).toBeVisible()

    // Back to the default: the time buckets are named for when a run is due,
    // never for an agent or a repository.
    await groupOption('Time').click()
    await expect(page.locator('.list-group-header').first()).toBeVisible()
    await expect(groupTitles.filter({ hasText: 'media-weekly' })).toHaveCount(0)
  })

  test('the group mode survives a page reload', async ({ page }) => {
    // usePersistedRef writes to real localStorage and reads it back on the
    // next mount - a jsdom-mocked storage in a unit test cannot exercise the
    // actual browser round-trip a reload performs, so this needs a real
    // navigation.
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await page.locator('.segmented-option').filter({ hasText: 'Repo' }).click()
    await expect(
      page.locator('.list-group-title').filter({ hasText: 'server-daily' }).first(),
    ).toBeVisible()

    await page.reload()
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.segmented-option').filter({ hasText: 'Repo' })).toHaveClass(
      /active/,
    )
    await expect(
      page.locator('.list-group-title').filter({ hasText: 'server-daily' }).first(),
    ).toBeVisible()
  })

  test('two agents writing to one repository at the same time get a notice', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // Seeded by seed-demo.sh: 'Offline agent due soon' (offline-due-01) and
    // 'Colliding daily window' (web-server-01) both write into server-daily,
    // the second starting while the first is still expected to run. Filtered
    // by agent rather than counted: other seeded pairs may clash too.
    const notice = page.locator('.notice-bar').filter({ hasText: 'offline-due-01 runs' })
    await expect(notice).toHaveCount(1)
    await expect(notice.locator('.notice-bar-title')).toHaveText(
      'Two agents write to repo server-daily at the same time',
    )
    await expect(notice).toContainText('web-server-01 starts')
    await expect(notice).toContainText('The repo lock makes one of them wait or fail.')

    // Both rows say who they clash with.
    await expect(
      healthChip(page.locator('.list-row', { hasText: 'Colliding daily window' })),
    ).toContainText('Overlaps offline-due-01')
    await expect(
      healthChip(page.locator('.list-row', { hasText: 'Offline agent due soon' })),
    ).toContainText('Overlaps web-server-01')

    // The notice opens the repository's schedules, where one can be moved.
    await notice.click()
    await expect(page).toHaveURL(/\/repos\/\d+\?tab=schedules$/)
  })

  test('the text filter scopes agent: and host: terms and combines them with a pipe', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const search = page.locator('input.search-input')
    // `.first()` for every presence assertion, for the reason scheduleRow()
    // documents above: earlier specs in the run add schedules of their own to
    // the shared demo, so more than one row can carry a given repo name and a
    // bare locator is one extra schedule away from a strict-mode violation.
    const cards = page.locator('.list-row')

    await search.fill('agent:media-store-01')
    await expect(cards.filter({ hasText: 'media-weekly' }).first()).toBeVisible()
    // database-hourly is db-server-01's alone, so scoping to another agent
    // drops it entirely.
    await expect(cards.filter({ hasText: 'database-hourly' })).toHaveCount(0)

    await search.fill('agent:media-store-01 | agent:db-server-01')
    await expect(cards.filter({ hasText: 'media-weekly' }).first()).toBeVisible()
    await expect(cards.filter({ hasText: 'database-hourly' }).first()).toBeVisible()

    // database-hourly and media-weekly live on localhost, so a host: term
    // keeps most of the list - and an agent hostname scoped to host: matches
    // nothing, which is what makes the two fields distinct.
    await search.fill('host:localhost')
    await expect(cards.first()).toBeVisible()

    await search.fill('host:media-store-01')
    await expect(cards).toHaveCount(0)

    // Both terms have to match: the agent keeps media-store-01's schedules, the
    // unknown host then rules every one of them out.
    await search.fill('agent:media-store-01 host:localhost')
    await expect(cards.filter({ hasText: 'media-weekly' }).first()).toBeVisible()

    await search.fill('agent:media-store-01 host:no-such-host')
    await expect(cards).toHaveCount(0)
  })

  test('the toolbar explains the filter syntax, including the pipe', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await page.getByRole('button', { name: 'Filter syntax' }).click()

    const dialog = page.getByRole('dialog')
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText('agent:k3s | agent:nas')
    await expect(dialog).toContainText('either may match')
    await expect(dialog).toContainText('Storage host the repository lives on')
  })

  test('overdue schedule row shows an Overdue chip with a per-host detail tooltip', async ({
    page,
  }) => {
    // The demo's seeded health data has no overdue hosts, so this reproduces
    // a host whose own last report is stale even though the schedule itself
    // looks on track, which is exactly what the Overdue chip's tooltip
    // exists to surface.
    const { overdueChip } = await openOverdueScheduleRow(page)
    await expect(overdueChip).toBeVisible()
    await expect(overdueChip).toContainText('Overdue')

    await expect(overdueChip).toHaveAttribute(
      'title',
      /Production Web Server \(web-server-01\) — last backup:/,
    )
  })

  test('running schedule row shows a Running chip', async ({ page }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, { last_status: 'started' })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // Same reasoning as scheduleRow(): this mocks schedule one's health, so
    // it has to assert against schedule one's row.
    const runningChip = healthChip(scheduleRow(page, 1))
    await expect(runningChip).toBeVisible()
    await expect(runningChip).toHaveText('Running')

    // Another schedule is not running. A healthy row shows no health label
    // at all, so ask for a Running label rather than reading one that may
    // not be there.
    const otherRow = page.locator('.list-row', { hasText: 'database-hourly' }).first()
    await expect(otherRow).toBeVisible()
    await expect(healthChip(otherRow).filter({ hasText: 'Running' })).toHaveCount(0)
  })

  // A row is a link to the schedule: its chips no longer navigate on their
  // own (the failed-run and warned-run links live on the detail page's Logs
  // tab), so clicking an overdue row is what opens the schedule.
  test('clicking an overdue schedule row opens the schedule detail page', async ({ page }) => {
    const { row, overdueChip } = await openOverdueScheduleRow(page)
    await expect(overdueChip).toBeVisible()

    await row.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/schedules\/1$/)
  })

  test("a failing schedule row shows a Failed chip carrying the run's error", async ({ page }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, {
      last_status: 'failed',
      last_error_message: 'Simulated failure',
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // Named in red text, not a pill: the row's dot already says it.
    const chip = healthChip(scheduleRow(page, 1))
    await expect(chip).toContainText('Failed')
    await expect(chip).toHaveClass(/schedules-health--danger/)
    await expect(chip).toHaveAttribute('title', /Simulated failure/)

    // The failure is read in the Activity log, so the label links there.
    await chip.click()
    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1&status=failed/)
  })

  test('a warned schedule row shows a Warning chip carrying the warning', async ({ page }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, {
      last_status: 'warning',
      last_error_message: 'Simulated warning',
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const chip = healthChip(scheduleRow(page, 1))
    await expect(chip).toContainText('Warning')
    await expect(chip).toHaveAttribute('title', /Simulated warning/)
  })

  test("schedule row shows a missed-backups warning below the schedule's threshold", async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, {
      consecutive_missed_backups: 1,
      missed_backup_threshold: 3,
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const row = scheduleRow(page, 1)
    await expect(healthChip(row)).toContainText('1/3 missed')

    await row.click()
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(/\/schedules\/1$/)
  })

  test('schedule row shows no missed-backups warning once the threshold is reached', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await mockScheduleOneHealth(page, {
      consecutive_missed_backups: 3,
      missed_backup_threshold: 3,
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await expect(healthChip(scheduleRow(page, 1))).not.toContainText('missed')
  })

  // The seed's config export/import round-trip leaves a second copy of every
  // schedule, so the demo's name matches two rows. Only the original carries a
  // pending marker - markers are per-target state, not configuration, so the
  // imported copy has none - which makes the chip itself the unambiguous handle
  // on the row these two tests mean.
  function catchUpPendingCard(page: Page): Locator {
    return page.locator('.list-row', { hasText: 'Catch-up pending' })
  }

  test('a schedule waiting on a host to come back shows a catch-up pending badge', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    const card = catchUpPendingCard(page)
    await expect(card).toHaveCount(1)
    await expect(card.getByText('Catch-up on reconnect demo')).toBeVisible()
  })

  test('the schedule Overview names the host a catch-up is waiting on', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await catchUpPendingCard(page).click()
    await page.waitForLoadState('networkidle')

    await expect(page.getByText(/Only if the next run is at least 2 hours away/)).toBeVisible()
    await expect(page.getByText(/Pending for/).first()).toBeVisible()
  })

  test('schedule detail General section saves its catch-up cutoff', async ({ page }) => {
    await loginAsAdmin(page)
    // The cutoff only applies to runs an agent or repository host marked as not
    // always online missed, and is disabled on a schedule that has none - so
    // this uses the seeded weekly schedule into the media-weekly NAS, which has
    // both.
    const scheduleId = await scheduleIdByName(page, 'Catch-up on an offline repository demo')
    await page.goto(`/schedules/${scheduleId}`)
    await page.waitForLoadState('networkidle')

    await page.getByRole('tab', { name: 'Settings' }).click()
    await openSettingsSection(page, 'General')

    // Whether a host is waited for is set on the host now, so the schedule has
    // no switch of its own - just the cutoff, and the machines it applies to:
    // the agent, and the repository host media-weekly lives on.
    await expect(page.getByRole('switch', { name: 'Catch up missed runs' })).toHaveCount(0)
    await expect(page.getByRole('link', { name: 'media-store-01' })).toBeVisible()
    await expect(page.getByRole('link', { name: 'localhost' })).toBeVisible()
    await expect(page.getByText('Catch-up cutoff', { exact: true })).toBeVisible()
    await expect(
      page.getByText(/A missed run is not caught up if this schedule runs again within 2 days/),
    ).toBeVisible()

    const lead = page.locator('#catch-up-lead')
    await expect(lead).toBeEnabled()
    // Seeded as two days, the cutoff a weekly schedule needs.
    await expect(lead).toHaveValue('2')
    await lead.fill('3')

    const waitForSave = await interceptScheduleSave(
      page,
      scheduleId,
      (requestBody, responseBody) => ({
        ...responseBody,
        catch_up_min_lead_minutes: requestBody.catch_up_min_lead_minutes,
      }),
    )

    await page.getByRole('button', { name: 'Save changes' }).click()

    const saved = await waitForSave()
    expect(saved.catch_up_min_lead_minutes).toBe(3 * 24 * 60)
    expect(saved).not.toHaveProperty('catch_up_missed_runs')
  })

  test('clicking a schedule navigates to detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // The row, not the first 'server-daily' on the page: an overlap notice
    // above the groups names the repository both runs write to, so a bare
    // text locator can pick up the notice instead of a schedule row.
    await page.locator('.list-row', { hasText: 'server-daily' }).first().click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/schedules\/\d+/)
  })

  // The detail page shows one schedule's hosts. Asking for the whole
  // installation's health made its first paint wait on a response that grows
  // with the number of schedule targets in the deployment.
  test('schedule detail asks for its own health and paints before it arrives', async ({ page }) => {
    await loginAsAdmin(page)
    // The dashboard that login lands on asks for the whole installation's
    // health. Let it settle before the route below starts recording, so the
    // only requests it sees are the schedule page's own.
    await page.waitForLoadState('networkidle')

    // Held open until the assertions below have run, so a page that still
    // waits on the health summary cannot render its Settings form.
    let releaseHealth: (() => void) | undefined
    const healthHeld = new Promise<void>((resolve) => {
      releaseHealth = resolve
    })
    let markRequested: (() => void) | undefined
    const healthRequested = new Promise<void>((resolve) => {
      markRequested = resolve
    })

    const healthUrls: string[] = []
    await page.route(
      (url) => url.pathname === '/api/stats/health',
      async (route) => {
        healthUrls.push(route.request().url())
        markRequested?.()
        await healthHeld
        await route.continue()
      },
    )

    await page.goto('/schedules/1')
    await healthRequested

    await page.getByRole('tab', { name: 'Settings' }).click()
    await expect(page.locator('.cron-input')).toHaveValue('0 2 * * *')

    expect(healthUrls).not.toHaveLength(0)
    for (const url of healthUrls) {
      expect(new URL(url).searchParams.get('schedule_id')).toBe('1')
    }

    releaseHealth?.()
    await page.waitForLoadState('networkidle')
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

  // The cron field is checked by the server's own validator and previewed by
  // the scheduler's own next-run calculation, so the form accepts exactly what
  // saving accepts (weekday names included) and rejects what saving rejects.
  test('schedule settings preview a cron expression with the scheduler', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.getByRole('tab', { name: 'Settings' }).click()
    const cron = page.locator('.cron-input')
    await expect(cron).toHaveValue('0 2 * * *')

    await cron.fill('0 2 * * MON-FRI')
    await expect(page.locator('.cron-error')).toHaveCount(0)
    await expect(page.locator('.next-run')).toHaveCount(3)

    await cron.fill('60 2 * * *')
    await expect(page.locator('.cron-error')).toContainText('invalid cron expression')
    await expect(page.locator('.next-runs')).toHaveCount(0)
  })

  // This schedule's Backups tab is an archive browser, and a failed run
  // wrote no archive - so the run detail under Recent runs shows its output
  // in place, and hands the run to the Activity log, opened on it.
  test('a failed run in the overview run detail opens on its host', async ({ page }) => {
    await loginAsAdmin(page)
    await page.route('**/api/schedules/1/reports**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reports: [makeFailedReport(9995, 1)], total: 1 }),
      }),
    )
    // Registered second so it wins for the more specific URL: the count
    // endpoint answers with an object, not a list of reports.
    await page.route('**/api/schedules/1/reports/failed/count**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 1 }),
      }),
    )
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // The only run is the newest, so it is the one picked.
    const pill = page.locator('.run-pill')
    await expect(pill).toHaveCount(1)
    await expect(pill).toHaveAttribute('aria-pressed', 'true')
    await expect(pill).toHaveClass(/run-tone--failed/)

    // Each host of the run is a log row that opens on its output.
    const detail = page.locator('.run-detail')
    const toggle = detail.locator('.log-row-summary').first()
    await toggle.click()
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
    await expect(detail.locator('.agent-row-detail')).toContainText('connection refused')

    await detail.getByRole('button', { name: 'Open in Activity log' }).click()

    await expect(page).toHaveURL(
      /\/activity\?category=backup&hostname=web-server-01&schedule_id=1&report=9995/,
    )
  })

  // There is no archive jump on the Overview any more - the Backups tab is
  // where archives are browsed. What the run detail owes a successful run is
  // which host ran it and how much it wrote.
  test('a successful run in the overview run detail lists its host and size', async ({ page }) => {
    await loginAsAdmin(page)
    const success = {
      ...makeFailedReport(9996, 1),
      status: 'success',
      error_message: null,
      original_size: 2_100_000_000,
      duration_secs: 401,
      archive_name: 'web-server-01-e2e',
      run_id: 'e2e-run-9996',
    }
    await page.route('**/api/schedules/1/reports**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reports: [success], total: 1 }),
      }),
    )
    await page.route('**/api/schedules/1/reports/failed/count**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 0 }),
      }),
    )
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.run-pill')).toHaveClass(/run-tone--success/)
    const rows = page.locator('.run-detail .log-row')
    await expect(rows).toHaveCount(1)
    await expect(rows.first().locator('.log-row-title')).toHaveText('Production Web Server')
    await expect(rows.first().locator('.log-row-facts')).toContainText('2.0 GB')
    await expect(rows.first().locator('.log-row-facts')).toContainText('6m 41s')
  })

  test('schedule detail shows retention policy', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await page.getByRole('tab', { name: 'Settings' }).click()
    const retention = await openSettingsSection(page, 'Retention')

    // The section card opens on its own lede, then a row per interval.
    await expect(page).toHaveURL(/section=retention/)
    await expect(retention).toContainText('borg keeps')
    await expect(retention.getByText('Daily', { exact: true })).toBeVisible()
    await expect(retention.getByText('Weekly', { exact: true })).toBeVisible()
  })

  test('schedule detail Advanced section edits and saves the hook command timeout', async ({
    page,
  }) => {
    await openScheduleAdvanced(page)

    const timeoutInput = await saveNumericScheduleField(
      page,
      'Hook command timeout',
      'hook_timeout_seconds',
      180,
      '#schedule-hook-timeout',
    )
    await expect(timeoutInput).toHaveValue('180')
  })

  test('schedule detail Advanced section adds a multi-line pre-backup command and saves it', async ({
    page,
  }) => {
    await openScheduleAdvanced(page)
    const { script: newRow } = await addPreBackupCommand(page)

    const script =
      'umount -l /mnt/pve/truenas-backup\npvesm status --storage truenas-backup || exit 1'
    await newRow.fill(script)

    expect(await savePreBackupCommands(page)).toContainEqual(
      expect.objectContaining({ command: script }),
    )

    // The multi-line script round-trips through the save intact, still in
    // its own field rather than getting flattened or split across rows.
    await expect(newRow).toHaveValue(script)
  })

  // The per-command timeout is what lets one slow hook (a hypervisor dump)
  // have a longer budget than the `systemctl stop` sitting beside it, instead
  // of every command on the schedule sharing the slowest one's.
  test('schedule detail Advanced section saves a per-command hook timeout', async ({ page }) => {
    await openScheduleAdvanced(page)
    const { field: preField, script: newRow } = await addPreBackupCommand(page)

    const script = 'vzdump --all 1 --storage backup-store'
    await newRow.fill(script)

    const timeoutInput = preField.locator('input[type="number"]').last()
    // Empty means "inherit", and the placeholder says which value that is.
    await expect(timeoutInput).toHaveValue('')
    await expect(timeoutInput).toHaveAttribute('placeholder', /\d+/)
    await timeoutInput.fill('7200')

    expect(await savePreBackupCommands(page)).toContainEqual({
      command: script,
      timeout_seconds: 7200,
    })

    await expect(timeoutInput).toHaveValue('7200')
  })

  // Include patterns rescue a path from a broader exclude - the happy path
  // for the feature covers typing one in and confirming it round-trips
  // through the save as `include_patterns_raw`, the same way exclude
  // patterns already do.
  test('schedule detail Advanced section edits and saves include patterns', async ({ page }) => {
    await openScheduleAdvanced(page)

    const includeField = page.getByLabel('Include patterns')
    await expect(includeField).toBeVisible()

    const waitForSave = await interceptScheduleSave(page, 1, (requestBody, responseBody) => ({
      ...responseBody,
      include_patterns_raw: requestBody.include_patterns_raw,
    }))

    await includeField.fill('/home/keep')
    await page.getByRole('button', { name: 'Save changes' }).click()

    const savedBody = await waitForSave()
    expect(savedBody.include_patterns_raw).toBe('/home/keep')
    await expect(includeField).toHaveValue('/home/keep')
  })

  test('schedule detail General section edits and saves the missed backup threshold', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await page.getByRole('tab', { name: 'Settings' }).click()

    const thresholdInput = await saveNumericScheduleField(
      page,
      'Mark as failed after',
      'missed_backup_threshold',
      5,
    )
    await expect(thresholdInput).toHaveValue('5')
  })

  test('schedule detail shows host and repository assignment', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Overview is the default tab: the info summary names the schedule's
    // hosts and its repository.
    const infoCard = page.locator('.panel', { hasText: 'Schedule info' })
    await expect(infoCard.getByText('Repository', { exact: true })).toBeVisible()
    await expect(page.getByText('server-daily').first()).toBeVisible()
    await expect(infoCard.getByText('Hosts', { exact: true })).toBeVisible()
    await expect(infoCard).toContainText('Production Web Server')
  })

  // The schedule has no Logs tab of its own any more: its run history is the
  // Activity log, filtered to it, reached from Recent runs. An old link to the
  // Logs tab lands there too.
  test("the schedule's run history is the Activity log, filtered to it", async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('tab', { name: /Logs/ })).toHaveCount(0)
    await page.getByRole('button', { name: 'More schedule actions' }).click()
    await expect(page.getByRole('menuitem', { name: 'Logs' })).toHaveCount(0)
    await page.getByRole('button', { name: 'More schedule actions' }).click()

    await page.getByRole('button', { name: 'View all in Activity log' }).click()
    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1$/)
    await expect(page.getByTestId('activity-entries').locator('.log-row').first()).toBeVisible()

    await page.goto('/schedules/1?tab=logs&status=failed')
    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1&status=failed$/)
  })

  /** Answers the Activity log with `count` runs of schedule 1, at most `limit` per request. */
  async function mockScheduleOneActivity(
    page: Page,
    count: number,
    status: (index: number) => string,
  ): Promise<void> {
    const entries = Array.from({ length: count }, (_, i) => ({
      id: 30_000 + i,
      hostname: 'web-server-01',
      target_name: 'server-daily',
      started_at: new Date(Date.now() - (i + 1) * 3_600_000).toISOString(),
      finished_at: new Date(Date.now() - (i + 1) * 3_600_000 + 60_000).toISOString(),
      status: status(i),
      duration_secs: 60,
      repo_id: 1,
      archive_name: null,
      error_message: status(i) === 'failed' ? 'connection refused' : null,
      schedule_id: 1,
      schedule_name: null,
      run_id: null,
      acknowledged: false,
    }))
    await page.route(
      (url) => url.pathname === '/api/stats/activity',
      (route) => {
        const limit = Number(new URL(route.request().url()).searchParams.get('limit') ?? '50')
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(entries.slice(0, limit)),
        })
      },
    )
  }

  // What the schedule's Failed label opens: its failed runs, and only those.
  test("the Activity log narrows the schedule's runs to its failures", async ({ page }) => {
    await loginAsAdmin(page)
    // The demo's own failures for this schedule age out as the demo date
    // moves, so the runs this filters are guaranteed here instead.
    await mockScheduleOneActivity(page, 4, (i) => (i % 2 === 0 ? 'failed' : 'success'))

    await page.goto('/activity?category=backup&schedule_id=1&status=failed')
    const rows = page.getByTestId('activity-entries').locator('.log-row')
    await expect(rows).toHaveCount(2)
    await expect(rows.locator('.log-row-dot.status-dot--danger')).toHaveCount(2)
  })

  test("the schedule's run history offers to load more past the first page", async ({ page }) => {
    await loginAsAdmin(page)
    await mockScheduleOneActivity(page, 60, () => 'success')

    await page.goto('/activity?category=backup&schedule_id=1')
    const rows = page.getByTestId('activity-entries').locator('.log-row')
    await expect(rows).toHaveCount(50)

    await page.getByRole('button', { name: 'Load more' }).click()
    await expect(rows).toHaveCount(60)
    await expect(page.getByRole('button', { name: 'Load more' })).toHaveCount(0)
  })

  // Failed run history has no archive behind it, so an admin should be able
  // to clear it out on demand rather than wait on the age-based retention
  // setting under System. The menu item and dialog count come from the
  // unbounded /reports/failed/count endpoint, not the report list above (a
  // page-size `limit` bounds that one) - mocked independently of whatever
  // the demo seed's own failure window currently contains, the same reason
  // agent-detail.spec.ts's equivalent test mocks both endpoints.
  test('clean up failed backups deletes failed report history for the schedule', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await page.route('**/api/schedules/1/reports**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reports: [makeFailedReport(9997, 1)], total: 1 }),
      }),
    )
    await page.route('**/api/schedules/1/reports/failed/count**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 1 }),
      }),
    )
    const countLoaded = page.waitForResponse(
      (res) => res.url().includes('/api/schedules/1/reports/failed/count') && res.status() === 200,
    )
    await page.reload()
    await countLoaded

    await page.getByRole('button', { name: 'More schedule actions' }).click()
    const cleanItem = page.getByRole('menuitem', { name: /^Clean up failed/ })
    await expect(cleanItem).toBeVisible()

    let deleteRequested = false
    await page.route('**/api/schedules/1/reports/failed**', async (route) => {
      deleteRequested = true
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ deleted: 1 }),
      })
    })

    await cleanItem.click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await page.getByRole('button', { name: 'Delete failed reports' }).click()

    await expect(page.getByText('Deleted 1 failed backup report.')).toBeVisible()
    expect(deleteRequested).toBe(true)
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
    await openSettingsSection(page, 'Targets')
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

  // The list has no switch of its own: a schedule is paused from its detail
  // page's Settings, and the list then files it under Disabled.
  test('disables a schedule from its Settings and the list files it under Disabled', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    // Snapshot the seeded row so the mocked PUT response below can echo back a
    // complete ScheduleResponse, without ever sending the PUT to the real
    // backend (which would mutate demo data other, possibly-parallel, tests rely on).
    const scheduleId = 1
    const getResp = await page.request.get(`/api/schedules/${scheduleId}`)
    expect(getResp.ok()).toBe(true)
    const original = (await getResp.json()) as Record<string, unknown>

    await page.goto(`/schedules/${scheduleId}`)
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.detail-header').getByText('Enabled')).toBeVisible()

    await page.route(`**/api/schedules/${scheduleId}`, async (route) => {
      if (route.request().method() !== 'PUT') return route.fallback()
      const body = (await route.request().postDataJSON()) as Record<string, unknown>
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ...original, ...body }),
      })
    })

    await page.getByRole('tab', { name: 'Settings' }).click()
    const general = await openSettingsSection(page, 'General')
    const toggle = general.getByRole('switch', { name: 'Enabled' })
    await expect(toggle).toHaveAttribute('aria-checked', 'true')
    await toggle.click()

    const [putResponse] = await Promise.all([
      page.waitForResponse(
        (resp) =>
          resp.url().endsWith(`/api/schedules/${scheduleId}`) && resp.request().method() === 'PUT',
      ),
      page.getByRole('button', { name: 'Save changes' }).click(),
    ])
    expect(putResponse.ok()).toBe(true)
    expect(putResponse.request().postDataJSON()).toMatchObject({ enabled: false })
    await expect(page.locator('.detail-header').getByText('Disabled')).toBeVisible()
  })

  test('the creation wizard gates each step and creates a multi-target schedule', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/new')
    await page.waitForLoadState('networkidle')

    // Scoped to the footer: the rail's Review step is a button too, captioned
    // "Create schedule", and getByRole matches an accessible name by substring
    // by default - so an unscoped locator finds the step, not the action.
    const foot = page.locator('.wizard-foot')
    const nextButton = foot.getByRole('button', { name: 'Continue' })
    const createButton = foot.getByRole('button', { name: 'Create schedule', exact: true })

    // Step 1 - the create action does not exist yet, and the step will not
    // advance while a mandatory field is empty. Pressing "Create schedule"
    // with half a form filled in is what the wizard exists to prevent.
    await expect(createButton).toHaveCount(0)
    await expect(nextButton).toBeDisabled()
    await page.locator('#schedule-name').fill('Wizard integrity check')
    // Integrity check, so the run does not depend on backup source paths.
    await page.locator('#schedule-type').selectOption({ label: 'Integrity check' })
    await expect(nextButton).toBeEnabled()
    await nextButton.click()

    // Step 2 - hosts.
    await expect(nextButton).toBeDisabled()
    await page.locator('.multi-select-trigger').click()
    await page.getByText('Production Web Server').click()
    await page.locator('.multi-select-trigger').click()
    await nextButton.click()

    // Step 3 - targets. One repository is preselected; add a second so the
    // schedule writes two independent copies.
    await expect(page.locator('.order-item')).toHaveCount(1)
    await page.getByRole('button', { name: 'Add repository' }).click()
    await expect(page.locator('.order-item')).toHaveCount(2)
    await nextButton.click()

    // Step 4 - Power. Nothing is required here: it defaults to each host's own
    // wake setting. Asserted by name so inserting another step fails here
    // rather than further along, where the symptom is a missing Create button.
    await expect(page.locator('.wizard-step--current')).toContainText('Power')
    await nextButton.click()

    // Step 5 - timing is prefilled, so straight on to Review.
    await expect(page.locator('.wizard-step--current')).toContainText('Timing')
    await nextButton.click()

    const [createResponse] = await Promise.all([
      page.waitForResponse(
        (resp) => resp.url().endsWith('/api/schedules') && resp.request().method() === 'POST',
      ),
      createButton.click(),
    ])
    expect(createResponse.ok()).toBe(true)
    const created = createResponse.request().postDataJSON()
    expect(created.repo_targets).toHaveLength(2)
    expect(created.repo_targets[0].required).toBe(true)

    // A successful create lands on the new schedule's detail page.
    await expect(page).toHaveURL(/\/schedules\/\d+$/)
    await expect(page.locator('.error-inline')).not.toBeVisible()

    // ...and the detail page's Targets section round-trips both repositories.
    const scheduleUrl = page.url()
    await page.goto(`${scheduleUrl}?tab=settings&section=targets`)
    await page.waitForLoadState('networkidle')
    await expect(page.locator('.order-item')).toHaveCount(2)
  })

  test('a schedule the scheduler auto-disabled for an unreachable agent shows why, not just that it is off', async ({
    page,
  }) => {
    // The scheduler itself flips these fields after repeated failures to reach
    // the schedule's target agent (see docs/agents.md) - simulated here via a
    // mocked list response rather than waiting out real backoff ticks.
    await loginAsAdmin(page)
    await mockScheduleOnePatch(page, {
      enabled: false,
      auto_disabled_agent_unreachable: true,
      consecutive_failures: 3,
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    // Several other seeded schedules also fall back to displaying the same
    // "server-daily" repo name (schedule 1 has no explicit name), so hasText
    // would match multiple rows - the schedule's own data-schedule-id is
    // the only unambiguous selector. A disabled schedule sits in the Disabled
    // group, which starts folded.
    await openDisabledGroup(page)
    const card = scheduleRow(page, 1)
    const statusPill = healthChip(card)
    await expect(statusPill).toBeVisible()
    await expect(statusPill).toHaveText('Auto-disabled · agent unreachable')

    await card.click()
    await expect(page).toHaveURL(/\/schedules\/\d+$/)
    // Scoped to the page's own identity header, not just getByText - the
    // same wording can also appear in an unrelated "other schedules for this
    // agent" widget elsewhere on the page.
    await expect(
      page.locator('.detail-header').getByText('Auto-disabled · agent unreachable'),
    ).toBeVisible()

    // Third surface: the agent's own Schedules tab, a table of its own (not
    // the list row or the detail header above), which says the schedule is
    // off where its next run would be.
    await page.goto('/agents/web-server-01?tab=schedules')
    await page.waitForLoadState('networkidle')
    const row = page.locator('.sched-row[data-schedule-id="1"]')
    await expect(row.locator('.sched-next')).toHaveText('Disabled')
  })

  test('a schedule auto-disabled by a local/config error (not an unreachable agent) shows the error variant', async ({
    page,
  }) => {
    // Same scheduler-driven state as the "agent unreachable" test above, but for
    // the other branch of scheduleDisabledLabel: a local/config failure (e.g. a
    // corrupted encrypted passphrase) counts toward the same threshold but is
    // never marked auto_disabled_agent_unreachable, since an unrelated agent
    // reconnect says nothing about whether the underlying data problem was fixed.
    await loginAsAdmin(page)
    await mockScheduleOnePatch(page, {
      enabled: false,
      auto_disabled_agent_unreachable: false,
      consecutive_failures: 3,
    })

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')

    await openDisabledGroup(page)
    const card = scheduleRow(page, 1)
    const statusPill = healthChip(card)
    await expect(statusPill).toBeVisible()
    await expect(statusPill).toHaveText('Auto-disabled · error')

    await card.click()
    await expect(page).toHaveURL(/\/schedules\/\d+$/)
    await expect(page.locator('.detail-header').getByText('Auto-disabled · error')).toBeVisible()

    await page.goto('/agents/web-server-01?tab=schedules')
    await page.waitForLoadState('networkidle')
    const row = page.locator('.sched-row[data-schedule-id="1"]')
    await expect(row.locator('.sched-next')).toHaveText('Disabled')
  })

  // schedule.last_run_at is dispatch bookkeeping, advanced only once a
  // trigger attempt reaches the agent - a run whose dispatch never got that
  // far (agent unreachable) can still settle a backup_reports row later
  // (e.g. abandoned as failed once the agent reconnects), leaving
  // last_run_at null while a completed backup genuinely exists. The agent's
  // own Schedules tab must read that completed-backup evidence from health
  // data instead of trusting last_run_at, and show the cron cadence in
  // plain English rather than the raw crontab string.
  test('agent Schedules tab shows real last-run evidence, not "never run", for a schedule with no last_run_at', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await mockScheduleOnePatch(page, { last_run_at: null })
    await mockScheduleOneHealth(page, {
      last_status: 'success',
      last_backup_at: '2020-01-01T02:00:00Z',
    })

    await page.goto('/agents/web-server-01?tab=schedules')
    await page.waitForLoadState('networkidle')

    // Several other seeded schedules also target the "server-daily" repo, so
    // a text filter would match multiple rows - the table row's
    // data-schedule-id is the only unambiguous selector.
    const row = page.locator('.sched-row[data-schedule-id="1"]')
    const last = row.locator('.sched-col-last')
    await expect(last).toContainText('Succeeded')
    await expect(last).toContainText(/\d+d ago/)
    await expect(last).not.toContainText('Never run')
    await expect(row.locator('.sched-col-cadence')).toHaveText('Daily at 02:00')
  })
})
