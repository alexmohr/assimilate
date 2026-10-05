// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { WebSocketRoute } from '@playwright/test'
import { expect, loginAsAdmin, mockRunningBackupOperation, test } from './fixtures'

test.describe('Dashboard — Backups In Progress panel', () => {
  test('shows the running backup with its schedule name and a running-for timer', async ({
    page,
  }) => {
    await mockRunningBackupOperation(page)

    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const panel = page.locator('.active-backups-panel')
    await expect(panel).toBeVisible()
    await expect(panel).toContainText('Backups in progress')
    await expect(panel.locator('.active-backup-schedule')).toContainText('server-daily')
    await expect(panel.locator('.active-backup-time').first()).toContainText('Running for')
  })

  test('the agent link in the panel navigates to that agent detail page', async ({ page }) => {
    await mockRunningBackupOperation(page)

    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const item = page.locator('.active-backup-item').first()
    await item.locator('.active-backup-link', { hasText: 'web-server-01' }).click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/agents\/web-server-01/)
  })

  test('the repo link in the panel navigates to that repository detail page', async ({ page }) => {
    await mockRunningBackupOperation(page)

    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const item = page.locator('.active-backup-item').first()
    // Two links share the same class (agent, repo); the repo link is the
    // second one and carries the repo name as its text.
    await item.locator('.active-backup-link', { hasText: 'server-daily' }).click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repos\//)
  })

  test('shows an estimated time remaining once historical duration data exists', async ({
    page,
  }) => {
    await mockRunningBackupOperation(page)

    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    // The demo seeds 30 days of daily backups for server-daily, so the ETA
    // fetch (last successful/warned runs for schedule 1 / repo 1) has real
    // history to average over.
    const item = page.locator('.active-backup-item').first()
    await expect(item.locator('.active-backup-time').last()).toContainText('left', {
      timeout: 10_000,
    })
  })

  test('shows a progress bar and swaps the waiting note for live progress once it streams in', async ({
    page,
  }) => {
    await mockRunningBackupOperation(page)
    // The UI socket is routed rather than connected, so the test decides when
    // the progress line arrives instead of waiting on a real borg run.
    // Every page load opens its own socket, so the newest one is the
    // dashboard's.
    const sockets: WebSocketRoute[] = []
    await page.routeWebSocket('**/ws/ui', (route) => {
      sockets.push(route)
    })

    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const item = page.locator('.active-backup-item', { hasText: 'server-daily' }).first()
    // The demo's history for this schedule gives an estimate, so the bar is
    // determinate and reports how far along the run is.
    await expect(item.getByRole('progressbar')).toHaveAttribute('aria-valuenow', /^\d+$/, {
      timeout: 10_000,
    })
    await expect(item.locator('.active-backup-progress-pending')).toBeVisible()
    await expect.poll(() => sockets.length).toBeGreaterThan(0)
    const ws = sockets[sockets.length - 1]

    const path = 'srv/data/photos/2026/img_0001.jpg'
    ws.send(
      JSON.stringify({
        type: 'BackupLog',
        payload: {
          hostname: 'web-server-01',
          repo_id: 1,
          schedule_id: 1,
          line: JSON.stringify({
            type: 'archive_progress',
            nfiles: 1234,
            original_size: 5_000_000,
            path,
          }),
        },
      }),
    )

    await expect(item.locator('.active-backup-progress-pending')).toHaveCount(0)
    await expect(item.locator('.active-backup-progress')).toContainText('1,234 files')
    const shown = item.locator('.active-backup-progress-path')
    await expect(shown).toHaveAttribute('title', path)
    await expect(shown.locator('.active-backup-progress-dir')).toHaveText('srv/data/photos/2026/')
    await expect(shown.locator('.active-backup-progress-file')).toHaveText('img_0001.jpg')
  })
})
