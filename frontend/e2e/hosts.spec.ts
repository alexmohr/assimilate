// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Route } from '@playwright/test'
import {
  expect,
  loginAsAdmin,
  mockRunningBackupOperation,
  mockScheduleOneHealth,
  test,
} from './fixtures'

test.describe('Hosts management', () => {
  test('hosts list shows connected agent hosts and imported placeholders', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await expect(page.getByText('web-server-01', { exact: true })).toBeVisible()
    await expect(page.getByText('db-server-01', { exact: true })).toBeVisible()
    await expect(page.getByText('media-store-01', { exact: true })).toBeVisible()
    await expect(page.getByText('old-webserver', { exact: true })).toBeVisible()
    await expect(page.getByText('legacy-db-prod', { exact: true })).toBeVisible()
  })

  test('hosts list shows the fleet summary band and a per-agent last-backup stat', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.fleet-summary')).toBeVisible()
    await expect(page.locator('.fleet-summary-counts')).toContainText('agent')

    const card = page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first()
    // The card states the freshest completed backup outright rather than
    // implying it through a fill; the coverage bar it replaces is gone.
    await expect(card.locator('.coverage-meter')).toHaveCount(0)
    // web-server-01's demo archives are written by borg directly, with no
    // scheduled run reported back, and `last_backup_at` comes from a
    // backup_reports row per (schedule, agent) - so 'Never' is the correct
    // reading here rather than a gap in the stat.
    await expect(card.locator('.stat').filter({ hasText: 'Last backup' })).toHaveText(/Never/)

    // stale-report-01 is the host the demo gives a real completed report
    // (backdated four days, see seed-demo.sh), so it is the one that
    // exercises the freshest-completed-backup path end to end.
    const reported = page.locator('.entity-card').filter({ hasText: 'stale-report-01' }).first()
    const lastBackup = reported.locator('.stat').filter({ hasText: 'Last backup' })
    await expect(lastBackup.locator('.stat-value')).toHaveText(/^\d+d ago$/)
  })

  test('hosts list groups its cards by the agent version each host reports', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    // The demo spans three groups: the build the live agent containers run,
    // the backdated 0.1.0 on the never-connected hosts, and the imported
    // placeholders that have never reported one.
    const groups = page.locator('.list-group')
    await expect(groups).toHaveCount(3)

    const titles = await groups.locator('.list-group-title').allTextContents()
    expect(titles).toContain('0.1.0')
    // Unknown always sorts last, however many builds are in front of it.
    expect(titles[titles.length - 1]).toBe('Unknown')

    const behind = groups.filter({ has: page.locator('.list-group-title', { hasText: '0.1.0' }) })
    await expect(behind.locator('.entity-card').filter({ hasText: 'offline-due-01' })).toBeVisible()

    // The demo server ships no arch-named agent binary, so it has no version
    // to compare against and the UI does not claim any group is behind - only
    // "never reported one" survives without that comparison.
    const unknown = groups.filter({
      has: page.locator('.list-group-title', { hasText: 'Unknown' }),
    })
    await expect(unknown.locator('.list-group-header .badge')).toHaveText('Never reported')

    // The version is the grouping now, so it is no longer a stat on the card.
    const card = page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first()
    await expect(card.locator('.stat').filter({ hasText: 'Agent' })).toHaveCount(0)
    await expect(card.locator('.badge--success')).toHaveText('Online')
  })

  test('fleet band splits the fleet into health segments', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const track = page.locator('.fleet-track')
    await expect(track).toBeVisible()
    await expect(track.locator('.fleet-seg').first()).toBeVisible()
    // Every segment the bar draws is named in the key below it.
    const segments = await track.locator('.fleet-seg').count()
    await expect(page.locator('.fleet-key .fleet-key-item')).toHaveCount(segments)
    await expect(page.locator('.fleet-key')).toContainText('offline')
  })

  test('clicking a host navigates to its detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first().click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\//)
    await expect(page.getByText('web-server-01').first()).toBeVisible()
  })

  test('deploy dialog opens and shows Load from remote button', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    // unassigned-01 is a placeholder with no backing container, so it never connects
    // and never reports an agent_version - unlike web-server-01, which runs a real
    // agent binary that reports its own version once connected (showing "Upgrade" or
    // nothing instead of "Deploy").
    const deployBtn = page
      .locator('.entity-card')
      .filter({ hasText: 'unassigned-01' })
      .locator('.card-actions button', { hasText: /Deploy|Upgrade/ })
      .first()
    await expect(deployBtn).toBeVisible({ timeout: 15_000 })
    await deployBtn.click()

    await expect(page.getByRole('heading', { name: /Deploy|Upgrade/ }).first()).toBeVisible()

    // The "Load from remote" button must be present - this was added in issue #124.
    const loadBtn = page.getByRole('button', { name: 'Load from remote' })
    await expect(loadBtn).toBeVisible()
    await expect(loadBtn).not.toBeDisabled()
  })

  test('agent card shows a Failed chip that navigates to the failed backup', async ({ page }) => {
    // Intercept the health API to inject a failure with an error message for web-server-01.
    await page.route('**/api/stats/health', async (route: Route) => {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify([
          {
            hostname: 'web-server-01',
            target_name: 'server-daily',
            last_status: 'failed',
            last_backup_at: new Date().toISOString(),
            is_overdue: false,
            last_error_message: 'Repository lock could not be acquired',
          },
        ]),
      })
    })

    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first()

    const failedChip = card.locator('.entity-issue-chip.sev-danger')
    await expect(failedChip).toBeVisible()
    await expect(failedChip).toContainText('failed')

    await failedChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\/web-server-01\?tab=backups&status=failed/)
  })

  test('two hosts can share a hostname as long as their domains differ', async ({ page }) => {
    const hostname = 'e2e-dup-host'
    const domainA = 'site-a.example.com'
    const domainB = 'site-b.example.com'

    async function addHost(domain: string): Promise<void> {
      await page.getByRole('button', { name: 'New' }).click()
      await page.getByPlaceholder('e.g. workstation-01').fill(hostname)
      await page.getByPlaceholder('Optional, e.g. lab.example.com').fill(domain)
      await page.getByRole('button', { name: 'Create' }).click()
      await expect(page.getByRole('heading', { name: 'Agent Created' })).toBeVisible()
      await page.getByRole('button', { name: 'Done' }).click()
    }

    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await addHost(domainA)
    await addHost(domainB)

    const cardA = page.locator('.entity-card').filter({ hasText: domainA })
    const cardB = page.locator('.entity-card').filter({ hasText: domainB })
    await expect(cardA).toHaveCount(1)
    await expect(cardB).toHaveCount(1)
    await expect(cardA.locator('.card-name')).toContainText(hostname)
    await expect(cardB.locator('.card-name')).toContainText(hostname)

    await cardA.click()
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(new RegExp(`/agents/${hostname}\\?domain=${domainA}`))
    await expect(page.locator('.crumb-current')).toHaveText(hostname)
    await expect(page.locator('.detail-breadcrumb .muted')).toHaveText(`(${domainA})`)

    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await cardB.click()
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(new RegExp(`/agents/${hostname}\\?domain=${domainB}`))
    await expect(page.locator('.detail-breadcrumb .muted')).toHaveText(`(${domainB})`)

    // Visiting the shared hostname without a domain is ambiguous - the UI
    // must offer a picker rather than silently resolving to either host.
    await page.goto(`/agents/${hostname}`)
    await page.waitForLoadState('networkidle')
    await expect(page.getByText(domainA)).toBeVisible()
    await expect(page.getByText(domainB)).toBeVisible()
  })

  test('agent card shows an Overdue chip that navigates to the schedules tab', async ({ page }) => {
    await page.route('**/api/stats/health', async (route: Route) => {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify([
          {
            hostname: 'web-server-01',
            target_name: 'server-daily',
            last_status: 'success',
            last_backup_at: '2020-01-01T02:00:00Z',
            is_overdue: true,
            last_error_message: null,
          },
        ]),
      })
    })

    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first()

    const overdueChip = card.locator('.entity-issue-chip.sev-warning')
    await expect(overdueChip).toBeVisible()
    await expect(overdueChip).toContainText('overdue')

    await overdueChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\/web-server-01\?tab=schedules&health=overdue/)
  })

  test('agent card shows a Running pill while a backup is in progress', async ({ page }) => {
    await mockRunningBackupOperation(page)

    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const card = page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first()
    const runningPill = card.locator('.entity-running-pill')
    await expect(runningPill).toBeVisible()
    await expect(runningPill).toContainText('server-daily')

    const otherCard = page.locator('.entity-card').filter({ hasText: 'db-server-01' }).first()
    await expect(otherCard.locator('.entity-running-pill')).not.toBeVisible()
  })

  test("agent detail schedules tab's Failed chip navigates to the filtered activity log", async ({
    page,
  }) => {
    // schedule 1 ("server-daily") targets web-server-01 - see schedules.spec.ts.
    // "Colliding daily window" targets the same agent/repo (see seed-demo.sh)
    // and can carry its own Failed chip (e.g. once its own seeded in-progress
    // report resolves), so a row/chip lookup scoped only by the shared repo
    // name "server-daily" is ambiguous - the accessible description below
    // (from last_error_message, unique to this mock) picks out schedule 1's
    // own chip regardless of what else is on the page.
    await mockScheduleOneHealth(page, {
      last_status: 'failed',
      last_error_message: 'Simulated failure',
    })

    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01?tab=schedules')
    await page.waitForLoadState('networkidle')

    const failedChip = page.getByRole('button', {
      name: 'Failed',
      description: 'Simulated failure',
    })
    await expect(failedChip).toBeVisible()

    await failedChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1&status=failed/)
  })
})
