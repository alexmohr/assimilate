// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page, Route } from '@playwright/test'
import {
  expect,
  loginAsAdmin,
  mockRunningBackupOperation,
  mockScheduleOneHealth,
  showEveryAgent,
  test,
} from './fixtures'

/** The Agents list row for `hostname`. */
function agentRow(page: Page, hostname: string): ReturnType<Page['locator']> {
  return page.locator('.list-row').filter({ hasText: hostname }).first()
}

test.describe('Hosts management', () => {
  test('hosts list shows connected agent hosts and imported placeholders', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await showEveryAgent(page)

    await expect(page.getByText('web-server-01', { exact: true })).toBeVisible()
    await expect(page.getByText('db-server-01', { exact: true })).toBeVisible()
    await expect(page.getByText('media-store-01', { exact: true })).toBeVisible()
    await expect(page.getByText('old-webserver', { exact: true })).toBeVisible()
    await expect(page.getByText('legacy-db-prod', { exact: true })).toBeVisible()
  })

  test('hosts list shows the fleet tiles and a per-agent last-backup column', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await showEveryAgent(page)

    const tiles = page.locator('.fleet-strip .fleet-cell')
    await expect(tiles).toHaveCount(4)
    await expect(tiles.locator('.stat-label')).toHaveText([
      'Agents',
      'Online',
      'Needs attention',
      'Versions',
    ])
    await expect(tiles.first().locator('.stat-sub')).toContainText('schedule')

    // web-server-01's own schedule (id 1) is the one backup-lifecycle.spec.ts
    // dispatches for real via Run now, so its Last backup stat is racy
    // against that spec's own dispatch/cancel timing - asserting on it here
    // would depend on e2e execution order. unassigned-01 is a placeholder
    // with no backing container (see the 'deploy dialog' test above) and no
    // schedule of its own, so nothing else in the suite ever gives it a
    // report; it is the deterministic host for exercising the 'Never'
    // rendering path.
    const neverReported = agentRow(page, 'unassigned-01')
    await expect(neverReported.locator('[data-col="last-backup"]')).toHaveText('Never')

    // stale-report-01 is the host the demo gives a real completed report
    // (backdated four days, see seed-demo.sh), so it is the one that
    // exercises the freshest-completed-backup path end to end.
    const reported = agentRow(page, 'stale-report-01')
    await expect(reported.locator('[data-col="last-backup"]')).toHaveText(/^\d+d ago$/)
  })

  test('hosts list groups its rows by the agent version each host reports', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await page.getByRole('radio', { name: 'Version' }).click()

    // Three groups: the version the live agent containers report (the agent
    // crate's own, so it is not hard-coded here), the 0.0.9 the seed backdates
    // onto the never-connected hosts, and the placeholders that have never
    // reported one at all.
    const groups = page.locator('.list-group')
    await expect(groups).toHaveCount(3)

    const titles = await groups.locator('.list-group-title').allTextContents()
    expect(titles).toContain('0.0.9')
    // Unknown always sorts last, however many builds are in front of it.
    expect(titles[titles.length - 1]).toBe('Unknown')

    // Every row lives in a group - grouping is the list, not an extra section
    // above an ungrouped remainder - and no version group folds any away.
    await expect(page.locator('.list-group .list-row')).toHaveCount(
      await page.locator('.list-row').count(),
    )
    await expect(page.locator('.list-more')).toHaveCount(0)

    const behind = groups.filter({ has: page.locator('.list-group-title', { hasText: '0.0.9' }) })
    await expect(behind.locator('.list-row').filter({ hasText: 'offline-due-01' })).toBeVisible()

    // The demo server ships no arch-named agent binary, so it has no version
    // to compare against and the UI does not claim any group is behind - only
    // "never reported one" survives without that comparison.
    const unknown = groups.filter({
      has: page.locator('.list-group-title', { hasText: 'Unknown' }),
    })
    await expect(unknown.locator('.list-group-header .badge')).toHaveText('Never reported')

    // The row still names its own version in the Version column.
    const row = agentRow(page, 'offline-due-01')
    await expect(row.locator('[data-col="version"]')).toHaveText('0.0.9')

    // The choice survives a reload.
    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(page.getByRole('radio', { name: 'Version' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
  })

  test('hosts list groups its rows by status, folding the offline group', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    // Status is the default grouping: what needs attention first, then the
    // healthy online hosts, then the offline ones with nothing due. Empty
    // groups are left out, so the titles are an ordered subset.
    const titles = await page.locator('.list-group-title').allTextContents()
    const order = ['Needs attention', 'Online', 'Offline']
    expect(titles.length).toBeGreaterThan(0)
    expect(titles).toEqual(order.filter((title) => titles.includes(title)))

    // The pills count each status and narrow the list to it.
    const attention = page.locator('.filter-pill', { hasText: 'Needs attention' })
    await expect(attention).toHaveText(/· \d+\s*$/)
    await attention.click()
    await expect(attention).toHaveClass(/active/)
    // unassigned-01 has no schedule at all, so the group is never empty.
    await expect(page.locator('.list-group-title')).toHaveText(['Needs attention'])
    await expect(agentRow(page, 'unassigned-01')).toBeVisible()
    await page.locator('.filter-pill', { hasText: 'All ·' }).click()

    // The demo has more than five offline hosts with nothing due, so the
    // Offline group shows five and folds the rest.
    const offline = page.locator('.list-group').filter({
      has: page.locator('.list-group-title', { hasText: 'Offline' }),
    })
    const more = offline.locator('.list-more')
    if ((await more.count()) > 0) {
      await expect(offline.locator('.list-row')).toHaveCount(5)
      await more.click()
      await expect(more).toHaveText('Show fewer')
      expect(await offline.locator('.list-row').count()).toBeGreaterThan(5)
      await more.click()
      await expect(offline.locator('.list-row')).toHaveCount(5)
    }
  })

  test('clicking a host navigates to its detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    // On the name, not the row's centre, which can land on an issue chip.
    await agentRow(page, 'web-server-01').locator('.list-row-name').click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\//)
    await expect(page.getByText('web-server-01').first()).toBeVisible()
  })

  test('the status filter survives a page reload', async ({ page }) => {
    // usePersistedRef writes to real localStorage and reads it back on the
    // next mount - a jsdom-mocked storage in a unit test cannot exercise the
    // actual browser round-trip a reload performs, so this needs a real
    // navigation.
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const offlinePill = page.locator('.filter-pill', { hasText: 'Offline' })
    await offlinePill.click()
    await expect(offlinePill).toHaveClass(/active/)
    await expect(page.getByText('web-server-01', { exact: true })).not.toBeVisible()

    await page.reload()
    await page.waitForLoadState('networkidle')

    await expect(offlinePill).toHaveClass(/active/)
    await expect(page.getByText('web-server-01', { exact: true })).not.toBeVisible()
  })

  test('deploy dialog opens and shows Load from remote button', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    // unassigned-01 is a placeholder with no backing container, so it never connects
    // and never reports an agent_version - unlike web-server-01, which runs a real
    // agent binary that reports its own version once connected (showing "Upgrade" or
    // nothing instead of "Deploy"). The action is the accented button in the
    // agent page's header; the list's rows only link there.
    await showEveryAgent(page)
    const row = agentRow(page, 'unassigned-01')
    await expect(row).toBeVisible({ timeout: 15_000 })
    await row.click()
    await expect(page).toHaveURL(/\/agents\/unassigned-01/)
    const deployBtn = page
      .locator('.detail-actions')
      .getByRole('button', { name: /^(Deploy|Upgrade) agent$/ })
    await expect(deployBtn).toBeVisible({ timeout: 15_000 })
    await deployBtn.click()

    await expect(page.getByRole('heading', { name: /Deploy|Upgrade/ }).first()).toBeVisible()

    // The "Load from remote" button must be present - this was added in issue #124.
    const loadBtn = page.getByRole('button', { name: 'Load from remote' })
    await expect(loadBtn).toBeVisible()
    await expect(loadBtn).not.toBeDisabled()
  })

  test('agent row shows a Failed chip that navigates to the failed backup', async ({ page }) => {
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

    const card = agentRow(page, 'web-server-01')

    const failedChip = card.locator('.entity-issue-chip.sev-danger')
    await expect(failedChip).toBeVisible()
    await expect(failedChip).toContainText('failed')

    await failedChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\/web-server-01\?tab=logs&status=failed/)
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

    // Narrowed to the new pair, so neither can be folded away in a long
    // Offline group.
    const search = page.getByPlaceholder('Filter by hostname or tag...')
    await search.fill(hostname)
    const cardA = page.locator('.list-row').filter({ hasText: domainA })
    const cardB = page.locator('.list-row').filter({ hasText: domainB })
    await expect(cardA).toHaveCount(1)
    await expect(cardB).toHaveCount(1)
    await expect(cardA.locator('.list-row-name')).toContainText(hostname)
    await expect(cardB.locator('.list-row-name')).toContainText(hostname)

    await cardA.locator('.list-row-name').click()
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(new RegExp(`/agents/${hostname}\\?domain=${domainA}`))
    await expect(page.locator('.crumb-current')).toHaveText(hostname)
    await expect(page.locator('.detail-breadcrumb .muted')).toHaveText(`(${domainA})`)

    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await search.fill(hostname)
    await cardB.locator('.list-row-name').click()
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

  test('agent row shows an Overdue chip that navigates to the schedules tab', async ({ page }) => {
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

    const card = agentRow(page, 'web-server-01')

    const overdueChip = card.locator('.entity-issue-chip.sev-warning')
    await expect(overdueChip).toBeVisible()
    await expect(overdueChip).toContainText('overdue')

    await overdueChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\/web-server-01\?tab=schedules&health=overdue/)
  })

  test('agent row shows a Running pill while a backup is in progress', async ({ page }) => {
    await mockRunningBackupOperation(page)

    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    const card = agentRow(page, 'web-server-01')
    const runningPill = card.locator('.entity-running-pill')
    await expect(runningPill).toBeVisible()
    await expect(runningPill).toContainText('server-daily')

    const otherCard = agentRow(page, 'db-server-01')
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
