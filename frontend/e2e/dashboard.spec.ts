// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

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
    await expect(panel).toContainText('Backups In Progress')
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
})
