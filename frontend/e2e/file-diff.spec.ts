// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

/**
 * Opens the newest seeded web-server-01 archive in server-daily at the
 * directory holding nginx.conf. The demo agent backs up the fixed
 * /tmp/web-server-01-src, and the newest of those archives changes
 * nginx.conf against the one before it (see start-agent.sh).
 */
async function openNginxDirectory(page: Page): Promise<void> {
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
  await page.getByPlaceholder('Search name or host').fill('web-server-01-backup')
  await page.locator('.archive-row-body:visible').first().click()

  const browserPanel = page.locator('.browser-panel').last()
  // Indexing an archive's contents on first access can take a while.
  await expect(browserPanel.locator('table.browser-table')).toBeVisible({ timeout: 120_000 })
  for (const dir of ['tmp', 'web-server-01-src', 'etc', 'nginx']) {
    await browserPanel.getByText(dir, { exact: true }).click()
    await expect(browserPanel.locator('.path-crumbs')).toContainText(dir)
  }
}

test.describe('Comparing a file between archives', () => {
  test('nginx.conf is compared with the archive before it, line by line', async ({ page }) => {
    await openNginxDirectory(page)

    await page.getByRole('button', { name: 'Compare nginx.conf with another archive' }).click()

    const dialog = page.getByRole('dialog', { name: 'Compare nginx.conf' })
    await expect(dialog).toBeVisible()
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
    await openNginxDirectory(page)
    const browserPanel = page.locator('.browser-panel').last()
    await browserPanel.getByText('conf.d', { exact: true }).click()
    await expect(browserPanel.locator('.path-crumbs')).toContainText('conf.d')

    await page.getByRole('button', { name: 'Compare default.conf with another archive' }).click()

    const dialog = page.getByRole('dialog', { name: 'Compare default.conf' })
    await expect(dialog.getByText('No differences')).toBeVisible({ timeout: 60_000 })
  })
})
