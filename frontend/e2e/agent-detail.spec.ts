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

  // A warned run still reached the archive step, so it should be reachable
  // from the agent view like a successful one - not stuck behind "Show
  // detail" with no way out to the archive browser.
  test('a warned run still links through to its archive', async ({ page }) => {
    await openAgent(page)
    await page.getByRole('tab', { name: /Backups/ }).click()
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

  test('settings is a fourth tab with its own sub-nav', async ({ page }) => {
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
