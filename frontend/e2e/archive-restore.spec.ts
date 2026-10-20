// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, openSeededWebArchive, test } from './fixtures'

/**
 * Opens web-server-01's Backups tab on its newest seeded archive, at the
 * folder the demo backs up (start-agent.sh: /tmp/tmp.web-server-01).
 */
async function openSeededArchive(page: Page): Promise<void> {
  await openSeededWebArchive(page)
  const browserPanel = page.locator('.browser-panel')
  for (const dir of ['tmp', 'tmp.web-server-01']) {
    await browserPanel.getByText(dir, { exact: true }).click()
    await expect(browserPanel.locator('.path-crumbs')).toContainText(dir)
  }
}

test.describe('Restoring picked files', () => {
  test('ticked files restore into a folder on the agent the dialog names', async ({ page }) => {
    await openSeededArchive(page)

    await page.getByRole('checkbox', { name: 'Select restore-example.txt' }).check()
    const bar = page.getByTestId('browser-selection')
    await expect(bar).toContainText('1 selected')
    await expect(bar).toContainText('to their original paths on web-server-01')

    await bar.getByRole('button', { name: 'Restore to web-server-01' }).click()
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

test.describe('Backups tab on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } })

  test('an opened archive replaces the list, and back returns to it', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01?tab=backups')
    await page.waitForLoadState('networkidle')

    const list = page.locator('.archives-panel')
    await expect(list).toBeVisible()
    await page.locator('.archive-row-body:visible').first().click()

    await expect(list).not.toBeVisible()
    await expect(page.locator('.browser-title')).toBeVisible()
    await page.getByRole('button', { name: 'Archives' }).click()
    await expect(list).toBeVisible()
  })
})
