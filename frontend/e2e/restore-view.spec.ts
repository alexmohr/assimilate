// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, seededIdByName, test } from './fixtures'
import type { Locator, Page } from '@playwright/test'

interface Archive {
  name: string
  start: string
}

/**
 * Opens the restore view on server-daily's newest web-server-01 backup, the
 * archive the seed indexes. Each backup holds one mktemp directory with
 * etc/nginx/conf.d/default.conf, var/www/html/{index.html,app.js} and
 * restore-example.txt under tmp/tmp.XXXXXXXXXX.
 */
async function openRestoreView(page: Page): Promise<void> {
  await page.goto('/')
  const repoId = await seededIdByName(page, '/api/repos', 'server-daily')
  const res = await page.request.get(`/api/repos/${repoId}/archives`)
  expect(res.ok()).toBeTruthy()
  const newest = ((await res.json()) as Archive[])
    .filter((a) => a.name.startsWith('web-server-01-backup-'))
    .sort((a, b) => b.start.localeCompare(a.start))[0]
  expect(newest, 'server-daily must hold a seeded web-server-01 backup').toBeDefined()

  await page.goto(`/restore?repo=${repoId}&archive=${encodeURIComponent(newest!.name)}`)
  await expect(treeItem(page, 'tmp')).toBeVisible({ timeout: 60_000 })
}

function treeItem(page: Page, path: string): Locator {
  return page.locator(`[role="treeitem"][data-path="${path}"]`)
}

/** The tree row whose path ends in `suffix`, as a search lists it. */
function treeItemEndingIn(page: Page, suffix: string): Locator {
  return page.locator(`[role="treeitem"][data-path$="${suffix}"]`)
}

/** Searches for `text` and ticks the matching file. */
async function searchAndTick(page: Page, text: string, suffix: string): Promise<void> {
  await page.getByTestId('restore-search').fill(text)
  const row = treeItemEndingIn(page, suffix)
  await expect(row).toBeVisible({ timeout: 15_000 })
  await expect(row.locator('mark')).toContainText(text)
  await row.locator('input[type="checkbox"]').click()
  await expect(row).toHaveAttribute('aria-checked', 'true')
}

test.describe('restore view', () => {
  test('search, tick files in two folders and restore them under a destination', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await openRestoreView(page)

    await searchAndTick(page, 'default.conf', 'etc/nginx/conf.d/default.conf')
    await searchAndTick(page, 'index.html', 'var/www/html/index.html')
    await page.getByTestId('restore-search').fill('')

    const basket = page.getByTestId('restore-basket')
    await expect(basket.locator('.basket-item')).toHaveCount(2)
    await expect(basket.getByTestId('basket-total')).toContainText('2 files')
    // Neither folder is ticked as a whole, so both show as partly selected.
    await expect(treeItem(page, 'tmp')).toHaveAttribute('aria-checked', 'mixed')

    await basket.getByLabel('Destination', { exact: true }).fill('/tmp/e2e-basket-restore')
    await basket.getByRole('button', { name: 'Restore to host' }).click()

    const dialog = page.getByRole('dialog')
    await expect(dialog.getByTestId('restore-confirm-destination')).toHaveText(
      '/tmp/e2e-basket-restore',
    )
    await expect(dialog).toContainText('overwritten')
    await dialog.getByRole('button', { name: 'Restore', exact: true }).click()

    const status = basket.getByTestId('basket-restore-status')
    await expect(status).toContainText('Restored', { timeout: 90_000 })
    await expect(status).toContainText('/tmp/e2e-basket-restore')
  })

  test('a ticked folder is one basket entry and downloads with a file as one tar.lz4', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await openRestoreView(page)

    await searchAndTick(page, 'restore-example', 'restore-example.txt')
    await page.getByTestId('restore-search').fill('conf.d')
    const confD = treeItemEndingIn(page, 'etc/nginx/conf.d')
    await expect(confD).toBeVisible({ timeout: 15_000 })
    await confD.locator('input[type="checkbox"]').click()

    const basket = page.getByTestId('restore-basket')
    await expect(basket.locator('.basket-item')).toHaveCount(2)
    await expect(basket.locator('.basket-item').filter({ hasText: 'conf.d' })).toContainText(
      '1 file',
    )

    const downloading = page.waitForEvent('download')
    await basket.getByTestId('basket-download').click()
    const download = await downloading
    expect(download.suggestedFilename()).toMatch(/\.tar\.lz4$/)
    expect(await download.failure()).toBeNull()
  })

  test('folders open with the keyboard and the space bar ticks them', async ({ page }) => {
    await loginAsAdmin(page)
    await openRestoreView(page)

    const tmp = treeItem(page, 'tmp')
    await tmp.focus()
    await page.keyboard.press('ArrowRight')
    await expect(tmp).toHaveAttribute('aria-expanded', 'true')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press(' ')

    const basket = page.getByTestId('restore-basket')
    await expect(basket.locator('.basket-item')).toHaveCount(1)
    await expect(tmp).toHaveAttribute('aria-checked', 'true')
    await page.getByRole('button', { name: 'Clear', exact: true }).first().click()
    await expect(basket.locator('.basket-item')).toHaveCount(0)
  })
})
