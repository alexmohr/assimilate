// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Locator, Page } from '@playwright/test'
import { expect, loginAsAdmin, openSeededWebArchive, test } from './fixtures'

/** Opens a folder of the open archive by clicking its name, and waits for the path to show it. */
async function openFolder(browserPanel: Locator, name: string): Promise<void> {
  await browserPanel.getByTestId('browser-entry').getByRole('button', { name, exact: true }).click()
  await expect(browserPanel.locator('.path-crumbs')).toContainText(name)
}

/** Goes up to the folder above the open one, through the path's crumbs. */
async function upOneLevel(browserPanel: Locator): Promise<void> {
  await browserPanel.locator('.path-crumbs .crumb').nth(-2).click()
}

/**
 * Opens web-server-01's Backups tab on its newest seeded archive, at the
 * folder the demo backs up (start-agent.sh: /tmp/tmp.web-server-01).
 */
async function openSeededArchive(page: Page): Promise<void> {
  await openSeededWebArchive(page)
  const browserPanel = page.locator('.browser-panel')
  for (const dir of ['tmp', 'tmp.web-server-01']) await openFolder(browserPanel, dir)
}

test.describe('Restoring picked files', () => {
  test('ticked files restore into a folder on the agent the dialog names', async ({ page }) => {
    await openSeededArchive(page)

    await page.getByRole('checkbox', { name: 'Select restore-example.txt' }).check()
    const bar = page.getByTestId('browser-selection')
    await expect(bar).toContainText('1 selected')
    await expect(bar).toContainText('Restores to web-server-01, at the original paths or elsewhere')

    await bar.getByTestId('restore-picked').click()
    const dialog = page.getByRole('dialog', { name: 'Where to restore' })
    await expect(dialog).toBeVisible()
    // It opens on the machine the backup came from, named as the agent list
    // names it: by its display name.
    await expect(dialog.getByRole('radio', { name: /Production Web Server/ })).toBeChecked()

    await dialog.getByRole('radio', { name: /Into a folder/ }).check()
    await dialog.getByLabel('Folder', { exact: true }).fill('/tmp/e2e-restore-dialog')
    await expect(dialog.getByTestId('restore-preview')).toContainText(
      '/tmp/e2e-restore-dialog/tmp/tmp.web-server-01/restore-example.txt',
    )

    await dialog.getByRole('button', { name: 'Restore onto Production Web Server' }).click()
    await expect(dialog).not.toBeVisible()
    // The restore runs in the background; its toast says it started.
    await expect(
      page
        .locator('.toast')
        .filter({ hasText: /restore/i })
        .first(),
    ).toBeVisible()
    await expect(bar).not.toBeVisible()
  })

  test('a folder that is not absolute cannot be restored into', async ({ page }) => {
    await openSeededArchive(page)
    await page.getByRole('checkbox', { name: 'Select restore-example.txt' }).check()
    await page.getByTestId('restore-picked').click()

    const dialog = page.getByRole('dialog', { name: 'Where to restore' })
    await dialog.getByRole('radio', { name: /Into a folder/ }).check()
    await dialog.getByLabel('Folder', { exact: true }).fill('relative/path')

    await expect(dialog).toContainText('Use an absolute path')
    await expect(dialog.getByTestId('restore-submit')).toBeDisabled()
  })
})

test.describe('Picking folders', () => {
  test('a ticked folder covers what is in it, and unticking one entry keeps the rest', async ({
    page,
  }) => {
    await openSeededWebArchive(page)
    const browserPanel = page.locator('.browser-panel')
    await openFolder(browserPanel, 'tmp')

    // What the folder holds comes from the content index: the demo source's
    // index.html, app.js, logo.png, nginx.conf, default.conf and
    // restore-example.txt.
    const folderRow = browserPanel
      .getByTestId('browser-entry')
      .filter({ hasText: 'tmp.web-server-01' })
    await expect(folderRow.getByTestId('folder-files')).toHaveText('6 files')

    const folderPick = page.getByRole('checkbox', { name: 'Select tmp.web-server-01' })
    await folderPick.check()
    const bar = page.getByTestId('browser-selection')
    await expect(bar).toContainText(/1 selected · .+ · 6 files/)

    await openFolder(browserPanel, 'tmp.web-server-01')
    const example = page.getByRole('checkbox', { name: 'Select restore-example.txt' })
    await expect(example).toBeChecked()
    await expect(page.getByRole('checkbox', { name: 'Select etc' })).toBeChecked()

    await example.uncheck()
    await expect(example).not.toBeChecked()
    await expect(page.getByRole('checkbox', { name: 'Select etc' })).toBeChecked()
    await expect(bar).toContainText(/2 selected · .+ · 5 files/)

    await upOneLevel(browserPanel)
    await expect(folderPick).not.toBeChecked()
    expect(await folderPick.evaluate((input: HTMLInputElement) => input.indeterminate)).toBe(true)
  })
})

test.describe('Searching an archive', () => {
  test('a search finds files anywhere in the archive, to pick or to open', async ({ page }) => {
    await openSeededWebArchive(page)
    const browserPanel = page.locator('.browser-panel')

    // The search sits behind the path bar's search button.
    await browserPanel.getByTestId('tree-search-toggle').click()
    await browserPanel.getByTestId('tree-search').fill('NGINX.conf')
    const hits = browserPanel.getByTestId('search-hit')
    await expect(hits).toHaveCount(1)
    // A hit names the file, and the folder it is in.
    await expect(
      hits.first().getByRole('checkbox', {
        name: 'Select tmp/tmp.web-server-01/etc/nginx/nginx.conf',
        exact: true,
      }),
    ).toBeVisible()
    await expect(hits.first()).toContainText('/tmp/tmp.web-server-01/etc/nginx')
    await expect(hits.first().locator('mark')).toHaveText('nginx.conf')

    // Only folders: etc/nginx and the conf.d folder in it match the text,
    // nginx.conf does not.
    await browserPanel.getByTestId('tree-search').fill('nginx')
    await browserPanel
      .getByRole('group', { name: 'Type' })
      .getByRole('button', { name: 'Folders' })
      .click()
    await expect(hits).toHaveCount(2)
    const folder = hits.filter({ hasNotText: 'conf.d' })
    await expect(
      folder.getByRole('checkbox', { name: 'Select tmp/tmp.web-server-01/etc/nginx', exact: true }),
    ).toBeVisible()

    await folder.getByTestId('pick-hit').check()
    await expect(page.getByTestId('browser-selection')).toContainText('1 selected')

    await folder.getByTestId('reveal-hit').click()
    await expect(browserPanel.getByTestId('tree-search')).toHaveValue('')
    await expect(browserPanel.locator('.path-crumbs')).toContainText('nginx')
    await expect(page.getByRole('checkbox', { name: 'Select nginx.conf' })).toBeChecked()
  })
})

test.describe('Change badges', () => {
  test('a file that differs from the previous archive is marked', async ({ page }) => {
    await openSeededWebArchive(page)
    const browserPanel = page.locator('.browser-panel')
    for (const dir of ['tmp', 'tmp.web-server-01', 'etc', 'nginx']) {
      await openFolder(browserPanel, dir)
    }

    // The demo writes every file afresh for each archive, so nginx.conf always
    // differs from the previous one. Reading that archive's folder may start
    // its indexing first; reopening the folder asks again once it is done.
    const row = browserPanel.getByTestId('browser-entry').filter({ hasText: 'nginx.conf' })
    await expect(async () => {
      await upOneLevel(browserPanel)
      await openFolder(browserPanel, 'nginx')
      await expect(row.getByTestId('change-badge')).toHaveText('changed', { timeout: 2_000 })
    }).toPass({ timeout: 120_000 })
    await expect(row.getByTestId('change-badge')).toHaveAttribute('title', /^changed since /)
  })
})

test.describe('Backups tab on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } })

  test('an opened archive replaces the list, and back returns to it', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01?tab=backups')
    await page.waitForLoadState('networkidle')

    const list = page.locator('.archives-panel')
    await expect(list).toBeVisible()
    await page.locator('[data-testid="archive-row"]:visible').first().click()

    await expect(list).not.toBeVisible()
    await expect(page.locator('.archive-head-name')).toBeVisible()
    await page.getByRole('button', { name: 'All archives' }).click()
    await expect(list).toBeVisible()
  })
})
