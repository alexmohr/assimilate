// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * The agent detail page: a persistent header, then Overview / Schedules /
 * Backups / Settings. Configuration lives behind the Settings tab so the
 * landing tab can answer the operational question instead.
 */

async function openAgent(page: Page, hostname = 'web-server-01'): Promise<void> {
  await loginAsAdmin(page)
  await page.goto(`/agents/${hostname}`)
  await page.waitForLoadState('networkidle')
}

test.describe('Agent detail', () => {
  test('overview answers is it up, did it work, when is the next run', async ({ page }) => {
    await openAgent(page)

    // The header is the identity, and it stays put across every tab.
    await expect(page.locator('.agent-hostname')).toHaveText('web-server-01')

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

    await expect(page.locator('.agent-menu-item')).toHaveCount(0)
    await page.locator('.agent-menu-toggle').click()

    await expect(page.locator('.agent-menu-item', { hasText: 'Edit identity' })).toBeVisible()
    await expect(page.locator('.agent-menu-item', { hasText: 'Regenerate token' })).toBeVisible()

    // Editing opens a dialog rather than an inline panel that reflows the page.
    await page.locator('.agent-menu-item', { hasText: 'Edit identity' }).click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.locator('input[placeholder="hostname"]')).toHaveValue('web-server-01')
  })

  test('backups are one line per run and expand their failure output', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: /Backups/ }).click()
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

  test('settings is a fourth tab with its own sub-nav', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: 'Settings' }).click()

    // A tab, not a route: the URL keeps the agent, and the header stays.
    await expect(page).toHaveURL(/\/agents\/web-server-01\?.*tab=settings/)
    await expect(page.locator('.agent-hostname')).toHaveText('web-server-01')

    await expect(page.locator('.settings-nav-item')).toHaveText([
      'Identity',
      'Backup defaults',
      'Hostname aliases',
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

  test('an imported host keeps every tab and explains the empty one', async ({ page }) => {
    await openAgent(page, 'old-webserver')

    await expect(page.locator('.badge', { hasText: 'Imported' })).toBeVisible()
    // Adoption takes the primary slot: an imported host has one job.
    await expect(page.getByRole('button', { name: 'Adopt' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Merge into...' })).toBeVisible()

    const tabs = page.getByRole('tab')
    await expect(tabs).toHaveCount(4)

    await page.getByRole('tab', { name: /Schedules/ }).click()
    await expect(page.locator('.empty-title')).toHaveText('No schedules yet')
    await expect(page.locator('.empty-description')).toContainText('reconstructed from archives')
  })
})
