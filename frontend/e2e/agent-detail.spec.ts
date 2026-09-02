// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, makeFailedReport, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * The agent detail page: a persistent header, then Overview / Schedules /
 * Backups / Logs / Settings. Backups is the archive browser; Logs is the
 * flat run history (any status, expandable warnings/errors) that used to
 * live under Backups. Configuration lives behind the Settings tab so the
 * landing tab can answer the operational question instead.
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
  test('overview answers is it up, did it work, when is the next run', async ({ page }) => {
    await openAgent(page)

    // The header is the identity, and it stays put across every tab.
    await expect(page.locator('.detail-name')).toHaveText('web-server-01')

    const tiles = page.locator('.tile .stat-label')
    await expect(tiles).toHaveText(['Last backup', 'Next run', 'Repositories', 'Recent runs'])

    // The outcome strip is the success tile: one cell per run, not a
    // percentage over a span of days that means something different at
    // every backup cadence.
    await expect(page.locator('.run-cell').first()).toBeVisible()
    await expect(page.locator('.run-strip-span')).toContainText('runs back to')
  })

  test('operational content is on the landing tab, configuration is not', async ({ page }) => {
    await openAgent(page)

    await expect(page.getByRole('heading', { name: 'Recent backups' })).toBeVisible()
    await expect(page.getByText('Backup defaults')).toHaveCount(0)
    await expect(page.getByText('Danger zone')).toHaveCount(0)
  })

  test('rare actions live behind the header overflow menu', async ({ page }) => {
    await openAgent(page)

    await expect(page.locator('.overflow-menu-item')).toHaveCount(0)
    await page.locator('.overflow-toggle').click()

    await expect(page.locator('.overflow-menu-item', { hasText: 'Edit identity' })).toBeVisible()
    await expect(page.locator('.overflow-menu-item', { hasText: 'Regenerate token' })).toBeVisible()

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

    await page.locator('.overflow-toggle').click()
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

  test('logs are one line per run and expand their failure output', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: /Logs/ }).click()
    await page.waitForLoadState('networkidle')

    const rows = page.locator('[id^="report-"]')
    await expect(rows.first()).toBeVisible()

    // The status filter doubles as a summary of the whole history.
    const failed = page.locator('.segmented-option', { hasText: /^Failed/ })
    await expect(failed).toBeVisible()
    await failed.click()

    const detailToggle = page.locator('button', { hasText: 'Show detail' }).first()
    if (await detailToggle.isVisible()) {
      await detailToggle.click()
      await expect(page.locator('.detail-output')).toBeVisible()
    }
  })

  // A warned run still reached the archive step, so it should be reachable
  // from the agent view like a successful one - not stuck behind "Show
  // detail" with no way out to the archive browser.
  test('a warned run still links through to its archive', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: /Logs/ }).click()
    await page.waitForLoadState('networkidle')

    const warning = page.locator('.segmented-option', { hasText: /^Warning/ })
    await expect(warning).toBeVisible()
    await warning.click()

    const openLink = page.locator('button.agent-row-name').first()
    await expect(openLink).toBeVisible()
    await openLink.click()

    await expect(page).toHaveURL(/\/repos\/\d+\?.*tab=archives/)
    await expect(page.locator('.archive-file-browser')).toBeVisible()
  })

  // A failed run in the Overview preview carries a badge and no output, so
  // the row is a dead end without this: the jump lands on the row that does
  // render the error, already expanded.
  test("the overview preview jumps to a failed run's error", async ({ page }) => {
    await loginAsAdmin(page)
    // The seed's own failures age out of the preview as the demo date moves,
    // so the run this asserts on is guaranteed here instead.
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

    await expect(page).toHaveURL(/\/agents\/web-server-01\?.*tab=logs.*report=9996/)
    await expect(page.locator('#report-9996.agent-row--highlighted')).toBeVisible()
    await expect(page.locator('.detail-output--danger')).toContainText('connection refused')
  })

  // The Backups tab used to be this same run log; it's the archive browser
  // now, consistent with a repository's and a schedule's own Backups tab.
  test('backups is the archive browser, grouped by repository', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: /^Backups/ }).click()
    await page.waitForLoadState('networkidle')

    await expect(page.locator('[id^="report-"]')).toHaveCount(0)
    await expect(page.getByText('server-daily').first()).toBeVisible()
    await expect(page.locator('.archive-row').first()).toBeVisible()
  })

  // db-server-01 also writes into server-daily via the multi-host schedule
  // (see seed-demo.sh), so its Backups tab has more than one repository
  // section - what makes the per-repository grouping visible at all.
  test('an agent backing up to more than one repository gets one archive section each', async ({
    page,
  }) => {
    await openAgent(page, 'db-server-01')
    await page.getByRole('tab', { name: /^Backups/ }).click()
    await page.waitForLoadState('networkidle')

    await expect(page.getByText('database-hourly').first()).toBeVisible()
    await expect(page.getByText('server-daily').first()).toBeVisible()
  })

  test('logs tab offers to load more once there are more runs than the first page', async ({
    page,
  }) => {
    // database-hourly backs up every hour, so db-server-01 has well over 50
    // runs in the demo seed - the one host guaranteed to exceed the first page.
    await openAgent(page, 'db-server-01')
    await page.getByRole('tab', { name: /Logs/ }).click()
    await page.waitForLoadState('networkidle')

    const loadMore = page.getByRole('button', { name: /^Load \d+ more$/ })
    await expect(loadMore).toBeVisible()
    const before = await page.locator('[id^="report-"]').count()

    await loadMore.click()
    await page.waitForLoadState('networkidle')

    await expect.poll(async () => page.locator('[id^="report-"]').count()).toBeGreaterThan(before)
  })

  test('settings is a fifth tab with its own sub-nav', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: 'Settings' }).click()

    // A tab, not a route: the URL keeps the agent, and the header stays.
    await expect(page).toHaveURL(/\/agents\/web-server-01\?.*tab=settings/)
    await expect(page.locator('.detail-name')).toHaveText('web-server-01')

    await expect(page.locator('.settings-nav-item')).toHaveText([
      'Identity',
      'Backup defaults',
      'Hostname aliases',
      'Power',
      'Tags',
      'Danger zone',
    ])

    // The four defaults cards became one card with five sections and one save.
    await page.locator('.settings-nav-item', { hasText: 'Backup defaults' }).click()
    await expect(page).toHaveURL(/section=defaults/)
    await expect(page.locator('.settings-pane .pane-lede')).toBeVisible()
    await expect(page.locator('.group-label')).toHaveText([
      'Backup paths',
      'Exclude patterns',
      'File change patterns',
      'Pre-backup commands',
      'Post-backup commands',
    ])
  })

  // A hook command is a whole script. Rendered inline the browser collapses
  // its newlines and indentation into single spaces, so the demo's multi-line
  // agent default arrived as one unreadable paragraph.
  test('Backup defaults shows a multi-line hook command with its lines and timeout', async ({
    page,
  }) => {
    await openAgent(page, 'media-store-01')
    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.locator('.settings-nav-item', { hasText: 'Backup defaults' }).click()

    const script = page.locator('.detail-pre').first()
    await expect(script).toBeVisible()
    await expect(script).toContainText('mountpoint -q /mnt/media')
    expect(await script.textContent()).toContain('\n')

    // The comment is coloured as one, which is the whole point of the block.
    await expect(script.locator('.sh-comment')).toContainText('#')

    // A command with its own timeout says so; one without says what it
    // inherits instead, so an empty value never reads as "no timeout".
    await expect(page.getByText('Timeout: 7200s')).toBeVisible()
    await expect(page.getByText("Timeout: the schedule's hook command timeout")).toBeVisible()
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

    await page.locator('.overflow-toggle').click()
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
