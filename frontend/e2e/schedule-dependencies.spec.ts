// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A schedule's dependencies - machines a backup needs besides its agent and
 * repository - from its Settings and its Overview. The demo seeds none, so the
 * dependency endpoints are answered here; everything else is the demo's own
 * "Nightly" schedule (id 1), which backs up web-server-01.
 */
const NAS = {
  id: 5,
  name: 'nas-media',
  address: 'nas-media.lan',
  port: 445,
  description: 'Media share',
  machine_id: 50,
  repo_host: null,
  intermittent: true,
  wake_enabled: false,
  last_checked_at: null,
  last_check_reachable: false,
  schedule_count: 1,
  agent_default_count: 0,
  waiting_count: 0,
}

async function mockDependencyHosts(page: Page): Promise<void> {
  await page.route('**/api/dependency-hosts', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([NAS]) }),
  )
}

test.describe('Schedule dependencies', () => {
  test('a schedule requires a dependency from its Settings', async ({ page }) => {
    await loginAsAdmin(page)
    await mockDependencyHosts(page)

    let saved: unknown = null
    await page.route('**/api/schedules/1/dependencies', async (route) => {
      if (route.request().method() === 'PUT') {
        saved = route.request().postDataJSON()
        const body = saved as { dependencies: { agent_id: number }[] }
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            dependencies: body.dependencies.map((d) => ({
              agent_id: d.agent_id,
              dependency_host_id: NAS.id,
              dependency_name: NAS.name,
              source: 'schedule',
              last_check_reachable: false,
            })),
            waiting: [],
          }),
        })
        return
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ dependencies: [], waiting: [] }),
      })
    })

    await page.goto('/schedules/1?tab=settings&section=power')
    await page.waitForLoadState('networkidle')
    await expect(
      page.getByRole('button', { name: 'Power & dependencies', exact: true }),
    ).toHaveAttribute('aria-current', 'true')
    // Below the schedule's power settings, which have an Edit of their own.
    const dependencies = page.locator('.settings-pane .pane-section', {
      hasText: 'Required dependencies',
    })
    await expect(dependencies.locator('.info-grid dd').first()).toHaveText('None')

    await dependencies.getByRole('button', { name: 'Edit' }).click()
    await page
      .getByRole('checkbox', { name: /nas-media/ })
      .first()
      .check()
    await dependencies.getByRole('button', { name: 'Save', exact: true }).click()

    await expect(dependencies.locator('.dependency-chip').first()).toContainText('nas-media')
    expect(saved).toMatchObject({ dependencies: [{ dependency_host_id: NAS.id }] })
  })

  test('the overview says what a skipped run waits for, and checks it', async ({ page }) => {
    await loginAsAdmin(page)
    await page.route('**/api/schedules/1/dependencies', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          dependencies: [
            {
              agent_id: 1,
              dependency_host_id: NAS.id,
              dependency_name: NAS.name,
              source: 'schedule',
              last_check_reachable: false,
            },
          ],
          waiting: [
            {
              schedule_id: 1,
              schedule_name: 'Nightly',
              agent_id: 1,
              hostname: 'web-server-01',
              dependency_host_id: NAS.id,
              dependency_name: NAS.name,
              pending_for: new Date(Date.now() - 3_600_000).toISOString(),
              last_probe_at: null,
              next_probe_at: new Date(Date.now() + 11 * 60_000).toISOString(),
              give_up_at: null,
              catching_up: false,
            },
          ],
        }),
      }),
    )
    // Check now asks the dependency's machine.
    await page.route(`**/api/dependency-hosts/${NAS.id}`, (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(NAS) }),
    )
    let checked = false
    await page.route(`**/api/machines/${NAS.machine_id}/check`, (route) => {
      checked = true
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ reachable: false, started: 0, abandoned: 0, dropped: 0 }),
      })
    })

    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.locator('.detail-header')).toContainText('Waiting for nas-media')
    const row = page.locator('.attention-row', { hasText: 'Waiting' })
    await expect(row).toContainText('Production Web Server was skipped at')
    await expect(row.getByRole('link', { name: 'nas-media' })).toHaveAttribute(
      'href',
      `/dependency-hosts/${NAS.id}`,
    )
    const info = page.locator('.panel', { hasText: 'Schedule info' })
    await expect(info.locator('.dependency-item')).toContainText('Not answering')

    await row.getByRole('button', { name: 'Check now' }).click()
    await expect(page.getByText('nas-media is still not answering')).toBeVisible()
    expect(checked).toBe(true)
  })
})
