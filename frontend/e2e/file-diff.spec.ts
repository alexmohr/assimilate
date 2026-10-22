// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, seededNginxComparison, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * Opens a seeded web-server-01 archive in server-daily at the directory
 * holding nginx.conf, and returns the archive to compare it with. The demo
 * agent backs up the fixed /tmp/tmp.web-server-01, and the two archives
 * `seededNginxComparison` picks hold different nginx.conf files.
 */
async function openNginxDirectory(page: Page): Promise<{ target: string; base: string }> {
  await loginAsAdmin(page)
  const repos = (await (await page.request.get('/api/repos')).json()) as {
    id: number
    name: string
  }[]
  const daily = repos.find((repo) => repo.name === 'server-daily')
  expect(daily, 'the seeded server-daily repository').toBeDefined()

  await page.goto(`/repos/${daily?.id}?tab=archives`)
  await page.waitForLoadState('networkidle')
  await page.getByText('Flat', { exact: true }).first().click()
  const { target, base } = await seededNginxComparison(page, daily?.id ?? 0)
  await page.getByPlaceholder('Search name, host or repository').fill(target)
  await page.locator('.archive-row-body:visible').first().click()

  const browserPanel = page.locator('.browser-panel').last()
  // Indexing an archive's contents on first access can take a while.
  await expect(browserPanel.locator('table.browser-table')).toBeVisible({ timeout: 120_000 })
  for (const dir of ['tmp', 'tmp.web-server-01', 'etc', 'nginx']) {
    await browserPanel.getByText(dir, { exact: true }).click()
    await expect(browserPanel.locator('.path-crumbs')).toContainText(dir)
  }
  return { target, base }
}

test.describe('Comparing a file between archives', () => {
  test('nginx.conf is compared with an older archive, line by line', async ({ page }) => {
    const { base } = await openNginxDirectory(page)

    await page.getByRole('button', { name: 'Compare nginx.conf with another archive' }).click()

    const dialog = page.getByRole('dialog', { name: 'Compare nginx.conf' })
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('file-diff-against').selectOption(base)
    const stats = dialog.getByTestId('file-diff-stats')
    await expect(stats).toContainText('-2', { timeout: 60_000 })
    await expect(stats).toContainText('+2')

    const body = dialog.getByTestId('file-diff-body')
    await expect(body.locator('.file-diff-line--removed')).toContainText([
      'worker_processes auto;',
      'client_max_body_size 64m;',
    ])
    await expect(body.locator('.file-diff-line--added')).toContainText([
      'worker_processes 4;',
      'include /etc/nginx/conf.d/rate-limit.conf;',
    ])
    await expect(body.locator('.file-diff-em').first()).toBeVisible()
    // Only the changes and their context: line 6 sits between the two hunks.
    await expect(body).not.toContainText('worker_connections 768;')

    await dialog.getByRole('switch', { name: 'Show the whole file' }).click()
    await expect(body).toContainText('worker_connections 768;', { timeout: 60_000 })

    await dialog.getByRole('radio', { name: 'Inline' }).click()
    await expect(dialog.locator('.file-diff-body--inline')).toBeVisible()

    await dialog.getByRole('button', { name: 'Close' }).click()
    await expect(dialog).not.toBeVisible()
  })

  test('a file that did not change says so', async ({ page }) => {
    const { base } = await openNginxDirectory(page)
    const browserPanel = page.locator('.browser-panel').last()
    await browserPanel.getByText('conf.d', { exact: true }).click()
    await expect(browserPanel.locator('.path-crumbs')).toContainText('conf.d')

    await page.getByRole('button', { name: 'Compare default.conf with another archive' }).click()

    const dialog = page.getByRole('dialog', { name: 'Compare default.conf' })
    await dialog.getByTestId('file-diff-against').selectOption(base)
    await expect(dialog.getByText('No differences')).toBeVisible({ timeout: 60_000 })
  })
})

test.describe('A file history', () => {
  /**
   * start-agent.sh leaves web-server-01's nginx.conf edited since its newest
   * archive, inside a directory its daily schedule backs up, so the agent
   * reads it over the encrypted channel when asked.
   */
  test('nginx.conf is compared with the file on its host now', async ({ page }) => {
    const { target } = await openNginxDirectory(page)
    // The seed restarts the server once, and agents wait a minute before
    // they reconnect: the file now can only be read once web-server-01 is back.
    await expect
      .poll(
        async () => {
          const agents = (await (await page.request.get('/api/agents')).json()) as {
            hostname: string
            is_connected: boolean
          }[]
          return agents.some((agent) => agent.hostname === 'web-server-01' && agent.is_connected)
        },
        { timeout: 120_000, intervals: [2_000] },
      )
      .toBe(true)

    await page.getByRole('button', { name: 'History of nginx.conf' }).click()

    const dialog = page.getByRole('dialog', { name: 'History of nginx.conf' })
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('file-history-live')).toContainText(
      'over the encrypted agent channel',
      { timeout: 30_000 },
    )
    // A real run may have written a newer archive; compare the seeded one.
    await dialog
      .locator(`[data-archives~="${target}"]`)
      .getByRole('button', { name: 'From' })
      .click()
    await dialog.getByTestId('file-history-to-live').click()
    await expect(dialog.getByTestId('file-history-to-label')).toHaveText('Now, on web-server-01')

    const body = dialog.getByTestId('file-diff-body')
    await expect(body.locator('.file-diff-line--added')).toContainText(
      ['worker_connections 2048;', 'gzip on;'],
      { timeout: 60_000 },
    )
    await expect(body.locator('.file-diff-line--removed')).toContainText([
      'worker_connections 768;',
    ])
    await expect(dialog.getByTestId('file-history-stats')).toContainText('+2')
  })

  test('indexing fills in the versions of every archive of the host', async ({ page }) => {
    const { target, base } = await openNginxDirectory(page)

    await page.getByRole('button', { name: 'History of nginx.conf' }).click()
    const dialog = page.getByRole('dialog', { name: 'History of nginx.conf' })
    // An earlier test may have indexed them already; otherwise start it.
    await expect(dialog.getByTestId('file-history-list')).toBeVisible({ timeout: 30_000 })
    const index = dialog.getByTestId('file-history-index')
    if ((await index.count()) > 0) {
      await index.click()
    }
    // Every seeded archive is indexed one after another.
    await expect(dialog.getByTestId('file-history-not_indexed')).toHaveCount(0, {
      timeout: 300_000,
    })

    await dialog.locator(`[data-archives~="${base}"]`).getByRole('button', { name: 'From' }).click()
    await dialog.locator(`[data-archives~="${target}"]`).getByRole('button', { name: 'To' }).click()
    const stats = dialog.getByTestId('file-history-stats')
    await expect(stats).toContainText('-2', { timeout: 60_000 })
    await expect(stats).toContainText('+2')
  })
})
