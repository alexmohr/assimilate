// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, makeFailedReport, openSettingsSection, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * The agent detail page: a persistent header, then Overview / Schedules /
 * Backups / Sources / Settings. Backups is the archive browser; the run
 * history the Logs tab used to hold is the Activity log, filtered to the
 * agent. Configuration lives behind the Settings tab so the landing tab can
 * answer the operational question instead.
 */

async function openAgent(page: Page, hostname = 'web-server-01'): Promise<void> {
  await loginAsAdmin(page)
  await page.goto(`/agents/${hostname}`)
  await page.waitForLoadState('networkidle')
}

// Minimal report row that satisfies AgentDetailView's "in progress" scan
// (status pending/started) - mirrors backup-lifecycle.spec.ts's makeReport.
function makeRunningReport(id: number): object {
  return {
    id,
    agent_id: 1,
    repo_id: 1,
    schedule_id: null,
    status: 'started',
    started_at: new Date().toISOString(),
    finished_at: new Date().toISOString(),
    original_size: 0,
    compressed_size: 0,
    deduplicated_size: 0,
    files_processed: 0,
    duration_secs: 0,
    error_message: null,
    warnings: [],
    borg_version: null,
    archive_name: null,
    borg_command: null,
    hostname: 'web-server-01',
    repo_name: 'server-daily',
    schedule_name: null,
  }
}

test.describe('Agent detail', () => {
  test('overview answers whether backups are OK, then shows the evidence', async ({ page }) => {
    await openAgent(page)

    // The header is the identity, and it stays put across every tab.
    await expect(page.locator('.detail-name')).toHaveText('web-server-01')

    // One verdict that is always there, then the last and next run.
    await expect(page.locator('.verdict')).toBeVisible()
    await expect(page.locator('.fact .stat-label')).toHaveText(['Last backup', 'Next run'])

    // One row per repository, each with its own state rather than one
    // summary mixing every repository's runs.
    const repoRow = page.locator('.repo-row').first()
    await expect(repoRow).toBeVisible()
    await expect(repoRow.locator('.repo-dot')).toBeVisible()
    await expect(repoRow.locator('.repo-meta')).not.toBeEmpty()
  })

  test('a repository row opens that repository', async ({ page }) => {
    await openAgent(page)

    // The name's link covers the row; a failed run's pill would open the run
    // instead, so the click goes to the name.
    await page.locator('.repo-row .repo-row-link').first().click()

    await expect(page).toHaveURL(/\/repos\/\d+/)
  })

  test('operational content is on the landing tab, configuration is not', async ({ page }) => {
    await openAgent(page)

    // Schedules and archives have tabs of their own, so the landing tab does
    // not preview them.
    await expect(page.getByRole('heading', { name: 'Repositories' })).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Recent backups' })).toHaveCount(0)
    await expect(page.getByText('Backup defaults')).toHaveCount(0)
    await expect(page.getByText('Danger zone')).toHaveCount(0)
  })

  test('rare actions live behind the header overflow menu', async ({ page }) => {
    await openAgent(page)

    await expect(page.locator('.overflow-menu-item')).toHaveCount(0)
    await page.getByRole('button', { name: 'More agent actions' }).click()

    await expect(page.locator('.overflow-menu-item', { hasText: 'Edit identity' })).toBeVisible()
    await expect(page.locator('.overflow-menu-item', { hasText: 'Regenerate token' })).toBeVisible()
    // Removing the host lives at the end of the menu, not in a Settings section.
    await expect(page.locator('.overflow-menu-item').last()).toHaveText('Delete agent')

    // Editing opens a dialog rather than an inline panel that reflows the page.
    await page.locator('.overflow-menu-item', { hasText: 'Edit identity' }).click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.locator('input[placeholder="hostname"]')).toHaveValue('web-server-01')
  })

  // An already-deployed agent has nothing in the primary header slot once it
  // is current, but the host it runs on may still need reinstalling - e.g.
  // after being reimaged - so Redeploy stays reachable through the menu.
  test('redeploy agent forces a deploy from the overflow menu', async ({ page }) => {
    await openAgent(page)

    await page.route(
      (url) => url.pathname === '/api/agents/web-server-01/service-unit',
      async (route) =>
        route.fulfill({ status: 200, contentType: 'application/json', body: '{"content":null}' }),
    )
    let deployBody: Record<string, unknown> | null = null
    await page.route(
      (url) => url.pathname === '/api/agents/web-server-01/deploy',
      async (route) => {
        deployBody = route.request().postDataJSON() as Record<string, unknown>
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ success: true, skipped: false, token: 'tok-redeploy-e2e' }),
        })
      },
    )

    await page.getByRole('button', { name: 'More agent actions' }).click()
    const redeployItem = page.locator('.overflow-menu-item', { hasText: 'Redeploy agent' })
    await expect(redeployItem).toBeVisible()
    await redeployItem.click()

    await expect(page.getByRole('heading', { name: /Redeploy Agent/ })).toBeVisible()
    await page.getByPlaceholder('e.g. 192.168.1.10').fill('10.0.0.9')
    await page.getByRole('button', { name: 'Redeploy Agent' }).click()

    await expect(page.locator('.deploy-success-msg')).toHaveText('Agent redeployed successfully.')
    await expect(page.locator('.token-text')).toHaveText('tok-redeploy-e2e')
    expect(deployBody).toMatchObject({ force: true, ssh_host: '10.0.0.9' })
  })

  // The agent page has no Logs tab of its own: its run history is the
  // Activity log, narrowed to the agent, one line per run that opens on its
  // output.
  test("the agent's runs are one line each in the Activity log and open their output", async ({
    page,
  }) => {
    await openAgent(page)
    await page.getByRole('button', { name: 'More agent actions' }).click()
    await page.getByRole('menuitem', { name: 'Activity log' }).click()

    await expect(page).toHaveURL(/\/activity\?.*hostname=web-server-01/)
    const rows = page.getByTestId('activity-entries').locator('.log-row')
    await expect(rows.first()).toBeVisible({ timeout: 15_000 })
    // Narrowed to the agent: every row is one of its runs.
    for (const title of await rows.locator('.log-row-title').allInnerTexts()) {
      expect(title.trim()).toBe('web-server-01')
    }

    await rows.first().locator('.log-row-summary').click()
    await expect(rows.first().locator('.log-row-detail')).toBeVisible()
  })

  // A failed run on the Overview carries no output, so the verdict would be a
  // dead end without this: the jump lands on the Activity log's row for the
  // run, which renders the error, already expanded.
  test("the overview verdict jumps to a failed run's error", async ({ page }) => {
    await loginAsAdmin(page)
    // The seed's own failures age out of the preview as the demo date moves,
    // so the run this asserts on is guaranteed here instead.
    const failed = makeFailedReport(9996) as Record<string, unknown>
    await page.route(
      (url) => url.pathname === '/api/stats/activity',
      (route) =>
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([
            {
              id: 9996,
              hostname: 'web-server-01',
              target_name: 'server-daily',
              started_at: failed.started_at,
              finished_at: failed.finished_at,
              status: 'failed',
              duration_secs: failed.duration_secs,
              repo_id: 1,
              archive_name: null,
              error_message: 'connection refused',
              schedule_id: null,
              schedule_name: null,
              run_id: null,
              acknowledged: false,
            },
          ]),
        }),
    )
    await page.route('**/api/agents/web-server-01/reports**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reports: [makeFailedReport(9996)], total: 1 }),
      }),
    )
    // Registered second so it wins for the more specific URL: the count
    // endpoint answers with an object, not a list of reports.
    await page.route('**/api/agents/web-server-01/reports/failed/count**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 1 }),
      }),
    )
    await page.goto('/agents/web-server-01')
    await page.waitForLoadState('networkidle')

    await page.getByRole('button', { name: 'View error' }).first().click()

    await expect(page).toHaveURL(/\/activity\?.*hostname=web-server-01.*report=9996/)
    const run = page.getByTestId('activity-entries').locator('.log-row--open')
    await expect(run).toHaveCount(1)
    await expect(run.locator('.error-pre')).toContainText('connection refused')
  })

  // The Backups tab used to be this same run log; it's the archive browser
  // now, consistent with a repository's and a schedule's own Backups tab.
  test('backups is the archive browser, grouped by repository', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: /^Backups/ }).click()
    await page.waitForLoadState('networkidle')

    await expect(page.locator('[id^="report-"]')).toHaveCount(0)
    await expect(page.getByText('server-daily').first()).toBeVisible()
    await expect(page.getByTestId('archive-row').first()).toBeVisible()
  })

  // db-server-01 also writes into server-daily via the multi-host schedule
  // (see seed-demo.sh), so its Backups tab spans more than one repository -
  // one list for both, each row tagged, with a filter to narrow it.
  test('an agent backing up to more than one repository gets one list across them', async ({
    page,
  }) => {
    await openAgent(page, 'db-server-01')
    await page.getByRole('tab', { name: /^Backups/ }).click()
    await page.waitForLoadState('networkidle')

    const list = page.locator('.archives-panel')
    await expect(list).toHaveCount(1)
    const filter = list.getByRole('group', { name: 'Repository' })
    await expect(filter).toBeVisible({ timeout: 30_000 })
    await expect(filter).toContainText('database-hourly')
    await expect(filter).toContainText('server-daily')

    // Under All, each row names its repository.
    const rows = list.getByTestId('archive-row')
    await expect(rows.filter({ hasText: 'database-hourly' }).first()).toBeVisible()

    // Narrowed to one repository, the list holds exactly that repository's
    // archives: as many rows as its pill counts.
    const serverDaily = filter.getByRole('button', { name: /server-daily/ })
    const count = Number(/(\d+)\s*$/.exec((await serverDaily.innerText()).trim())?.[1])
    expect(count).toBeGreaterThan(0)
    await serverDaily.click()
    await expect(serverDaily).toHaveAttribute('aria-pressed', 'true')
    await expect(rows).toHaveCount(count)
    await expect(rows.filter({ hasText: 'database-hourly' })).toHaveCount(0)
  })

  test("the agent's run history offers to load more past the first page", async ({ page }) => {
    // seed-demo.sh backfills db-server-01's hourly history well past 50 runs,
    // the one host guaranteed to exceed the Activity log's first page.
    await openAgent(page, 'db-server-01')
    await page.getByRole('button', { name: 'More agent actions' }).click()
    await page.getByRole('menuitem', { name: 'Activity log' }).click()
    await expect(page).toHaveURL(/\/activity\?.*hostname=db-server-01/)

    const rows = page.getByTestId('activity-entries').locator('.log-row')
    await expect(rows.first()).toBeVisible({ timeout: 15_000 })
    const loadMore = page.getByRole('button', { name: 'Load more' })
    await expect(loadMore).toBeVisible()
    const before = await rows.count()

    await loadMore.click()

    await expect.poll(async () => rows.count()).toBeGreaterThan(before)
  })

  test('settings is a tab of collapsible sections', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: 'Settings' }).click()

    // A tab, not a route: the URL keeps the agent, and the header stays.
    await expect(page).toHaveURL(/\/agents\/web-server-01\?.*tab=settings/)
    await expect(page.locator('.detail-name')).toHaveText('web-server-01')

    // Virtual machines and databases moved to the Sources tab, tags to the header.
    await expect(page.locator('.accordion-head')).toHaveText([
      'Identity',
      'Backup defaults',
      'Power & dependencies',
    ])

    // The four defaults cards became one card with five sections and one save.
    const defaults = await openSettingsSection(page, 'Backup defaults')
    await expect(page).toHaveURL(/section=defaults/)
    await expect(defaults).toContainText(
      'What a schedule uses for this host when it does not set its own paths',
    )
    await expect(defaults.locator('.group-label')).toHaveText([
      'Backup paths',
      'Exclude patterns',
      'File change patterns',
      'Pre-backup commands',
      'Post-backup commands',
    ])

    // Required dependencies live with the machine's power settings now.
    const power = await openSettingsSection(page, 'Power & dependencies')
    await expect(power.locator('.group-label', { hasText: 'Required dependencies' })).toBeVisible()
  })

  test('virtual machines and databases are the Sources tab', async ({ page }) => {
    await openAgent(page, 'db-server-01')
    await page.getByRole('tab', { name: 'Sources' }).click()

    await expect(page).toHaveURL(/\/agents\/db-server-01\?.*tab=sources/)
    await expect(page.locator('.accordion-head')).toHaveText(['Virtual machines', 'Databases'])
    const databases = await openSettingsSection(page, 'Databases')
    await expect(page).toHaveURL(/section=databases/)
    await expect(databases).toContainText('shop/orders-db')
  })

  // A hook command is a whole script. Rendered inline the browser collapses
  // its newlines and indentation into single spaces, so the demo's multi-line
  // agent default arrived as one unreadable paragraph.
  test('Backup defaults shows a multi-line hook command with its lines and timeout', async ({
    page,
  }) => {
    await openAgent(page, 'media-store-01')
    await page.getByRole('tab', { name: 'Settings' }).click()
    const defaults = await openSettingsSection(page, 'Backup defaults')

    const script = defaults.locator('.detail-pre').first()
    await expect(script).toBeVisible()
    await expect(script).toContainText('mountpoint -q /mnt/media')
    expect(await script.textContent()).toContain('\n')

    // The comment is coloured as one, which is the whole point of the block.
    await expect(script.locator('.sh-comment')).toContainText('#')

    // A command with its own timeout says so; one without says what it
    // inherits instead, so an empty value never reads as "no timeout".
    await expect(defaults.getByText('Timeout: 7200s')).toBeVisible()
    await expect(defaults.getByText("Timeout: the schedule's hook command timeout")).toBeVisible()
  })

  test('an imported host keeps every tab and explains the empty one', async ({ page }) => {
    await openAgent(page, 'old-webserver')

    await expect(page.locator('.badge', { hasText: 'Imported' })).toBeVisible()
    // Adoption takes the primary slot: an imported host has one job.
    await expect(page.getByRole('button', { name: 'Adopt' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Merge into...' })).toBeVisible()

    const tabs = page.getByRole('tab')
    await expect(tabs).toHaveCount(5)

    await page.getByRole('tab', { name: /Schedules/ }).click()
    await expect(page.locator('.empty-title')).toHaveText('No schedules yet')
    await expect(page.locator('.empty-description')).toContainText('reconstructed from archives')
  })

  test('running backup links its repository and Cancel backup sends the request', async ({
    page,
  }) => {
    await openAgent(page)

    await page.route('**/api/agents/web-server-01/reports**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reports: [makeRunningReport(9998)], total: 1 }),
      }),
    )
    await page.reload()

    const badge = page.locator('.live-log-host-badge')
    await expect(badge).toBeVisible({ timeout: 10_000 })
    await expect(badge).toHaveAttribute('href', '/repos/1')

    const cancelBtn = page.getByRole('button', { name: 'Cancel backup' })
    await expect(cancelBtn).toBeVisible()
    await cancelBtn.click()

    await expect(page.getByText(/cancel request sent/i)).toBeVisible({ timeout: 5_000 })
  })

  // Failed run history has no archive behind it, so an admin should be able
  // to clear it out on demand rather than wait on the age-based retention
  // setting under System. Reachable from the header overflow menu, like every
  // other rare/destructive agent action.
  test('clean up failed backups deletes failed report history for the agent', async ({ page }) => {
    await openAgent(page)

    // The demo seed's own failure window shifts with the current date, so
    // the menu item's visibility can't depend on it - guarantee one here.
    await page.route('**/api/agents/web-server-01/reports**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reports: [makeFailedReport(9997)], total: 1 }),
      }),
    )
    // The menu label and dialog count come from a separate, unbounded count
    // endpoint (not from the report list above, which a page-size `limit`
    // bounds) - registered after the broader route above so it wins for
    // this more specific URL.
    await page.route('**/api/agents/web-server-01/reports/failed/count**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 1 }),
      }),
    )
    // networkidle doesn't reliably gate on this fetch resolving, and the
    // menu item's count comes from it - wait for the response itself so the
    // menu isn't opened before the count has a chance to update.
    const countLoaded = page.waitForResponse(
      (res) =>
        res.url().includes('/api/agents/web-server-01/reports/failed/count') &&
        res.status() === 200,
    )
    await page.reload()
    await countLoaded

    await page.getByRole('button', { name: 'More agent actions' }).click()
    // getByRole's accessible name is whitespace-normalized; a plain
    // `.overflow-menu-item` locator with a `hasText` regex tests the raw
    // (unnormalized) textContent instead, so the template's leading space
    // before "Clean" breaks a `^`-anchored pattern - see the identical,
    // working locator in schedules.spec.ts's equivalent test.
    const cleanItem = page.getByRole('menuitem', { name: /^Clean up failed/ })
    await expect(cleanItem).toBeVisible()

    let deleteRequested = false
    await page.route('**/api/agents/web-server-01/reports/failed**', async (route) => {
      deleteRequested = true
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ deleted: 2 }),
      })
    })

    await cleanItem.click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await page.getByRole('button', { name: 'Delete failed reports' }).click()

    await expect(page.getByText('Deleted 2 failed backup reports.')).toBeVisible()
    expect(deleteRequested).toBe(true)
  })
})
