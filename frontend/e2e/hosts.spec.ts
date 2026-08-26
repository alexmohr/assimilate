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

  test('hosts list shows the fleet summary band and a per-agent coverage meter', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.fleet-summary')).toBeVisible()
    await expect(page.locator('.fleet-summary-counts')).toContainText('agent')

    const card = page.locator('.entity-card').filter({ hasText: 'web-server-01' }).first()
    await expect(card.locator('.coverage-meter')).toBeVisible()
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
    await mockScheduleOneHealth(page, {
      last_status: 'failed',
      last_error_message: 'Simulated failure',
    })

    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01?tab=schedules')
    await page.waitForLoadState('networkidle')

    const row = page.locator('.rows .agent-row').filter({ hasText: 'server-daily' })
    const failedChip = row.locator('.entity-issue-chip.sev-danger')
    await expect(failedChip).toBeVisible()

    await failedChip.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/activity\?category=backup&schedule_id=1&status=failed/)
  })
})
