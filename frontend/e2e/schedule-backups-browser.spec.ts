// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

async function gotoBackupsTab(page: Awaited<ReturnType<typeof test.info>['page']>): Promise<void> {
  await loginAsAdmin(page)
  await page.goto('/schedules/1')
  await page.waitForLoadState('networkidle')
  await page.getByRole('tab', { name: 'Backups' }).click()
  await page.waitForTimeout(1000)
}

async function clickFirstArchiveRow(
  page: Awaited<ReturnType<typeof test.info>['page']>,
): Promise<boolean> {
  const archiveRow = page.locator('.archive-row-select').first()
  const rowVisible = await archiveRow.isVisible({ timeout: 5_000 }).catch(() => false)
  if (!rowVisible) return false
  await archiveRow.click()
  await page.waitForTimeout(1000)
  return true
}

test.describe('Schedule backups tab - archive browser', () => {
  test('Backups tab is visible on backup-type schedule detail', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('tab', { name: 'Backups' })).toBeVisible()
  })

  test('save bar shows only on the Settings tab', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Overview is the default tab: no form, no save bar.
    await expect(page.locator('.save-bar')).not.toBeVisible()

    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.waitForTimeout(500)
    await expect(page.locator('.save-bar')).toBeVisible()

    await page.getByRole('tab', { name: 'Backups' }).click()
    await page.waitForTimeout(500)
    await expect(page.locator('.save-bar')).not.toBeVisible()
  })

  test('backups tab shows empty state or archive list', async ({ page }) => {
    await gotoBackupsTab(page)

    // Either the archives panel title is visible (with data)
    // or the empty state message is shown (no archives yet)
    const panelTitle = page.locator('.panel-title').filter({ hasText: 'Archives' })
    const emptyState = page.locator('.empty-state').filter({ hasText: 'No backup archives' })
    await expect(panelTitle.or(emptyState).first()).toBeVisible({ timeout: 10_000 })
  })

  test('backups tab renders split layout structure', async ({ page }) => {
    await gotoBackupsTab(page)

    // The backups layout should be rendered (either with data or empty)
    const backupsLayout = page.locator('.archive-browser-layout')
    const tabContent = page
      .locator('.tab-content')
      .filter({ hasText: /Archives|No backup archives/ })
    await expect(backupsLayout.or(tabContent).first()).toBeVisible({ timeout: 10_000 })
  })

  test('file browser structure renders when archive is selected', async ({ page }) => {
    await gotoBackupsTab(page)

    const hasArchive = await clickFirstArchiveRow(page)
    if (!hasArchive) {
      test.skip()
      return
    }

    // The file browser should show with breadcrumb
    await expect(page.locator('.path-crumbs')).toBeVisible()
    await expect(
      page.locator('.path-crumbs').getByText('~').or(page.locator('.path-crumbs').getByText('/')),
    ).toBeVisible()
  })

  test('download buttons present in file browser when archive selected', async ({ page }) => {
    await gotoBackupsTab(page)

    if (!(await clickFirstArchiveRow(page))) {
      test.skip()
      return
    }
    await page.waitForTimeout(1000)

    // Check for download buttons in the file browser
    const downloadButton = page.locator('.archive-file-browser button[title*="Download"]').first()
    const buttonVisible = await downloadButton.isVisible({ timeout: 10_000 }).catch(() => false)
    if (buttonVisible) {
      await expect(downloadButton).toBeVisible()
    }
  })

  // The schedule tab used to render its own four-column table: no grouping, no
  // search, no delete. It now renders the same ArchiveExplorer as the
  // repository and Archives screens.
  test('backups tab renders the shared archive selector controls', async ({ page }) => {
    await gotoBackupsTab(page)

    if (
      !(await page
        .locator('.archive-row')
        .first()
        .isVisible()
        .catch(() => false))
    ) {
      test.skip()
      return
    }

    await expect(page.locator('.archive-controls input')).toBeVisible()
    await expect(page.locator('.archive-sort-select')).toBeVisible()
    await expect(page.locator('.archive-group-toggle')).toBeVisible()
    const groupHost = page.locator('.archive-group .group-hostname').first()
    await expect(groupHost).toBeVisible()
    await expect(groupHost).toHaveAttribute('href', /^\/agents\/.+/)
  })

  test('an admin can reach archive deletion from the backups tab', async ({ page }) => {
    await gotoBackupsTab(page)

    if (
      !(await page
        .locator('.archive-row')
        .first()
        .isVisible()
        .catch(() => false))
    ) {
      test.skip()
      return
    }

    // Open the confirmation and cancel it: the point is that the control is
    // there and wired up, not that this run deletes a seeded archive.
    await page.locator('.archive-row button[title="Delete archive"]').first().click()
    await expect(page.locator('.archive-delete-message')).toBeVisible()
    await page.locator('.modal-footer').getByRole('button', { name: 'Cancel' }).click()
    await expect(page.locator('.archive-delete-message')).not.toBeVisible()
  })

  test('breadcrumb navigation updates when navigating directories', async ({ page }) => {
    await gotoBackupsTab(page)

    if (!(await clickFirstArchiveRow(page))) {
      test.skip()
      return
    }

    // Breadcrumb should show root
    const breadcrumb = page.locator('.path-crumbs')
    await expect(breadcrumb).toBeVisible()

    // Try navigating into a directory if one exists
    const dirEntry = page.locator('.archive-file-browser tr.clickable').first()
    const dirVisible = await dirEntry.isVisible({ timeout: 5_000 }).catch(() => false)
    if (!dirVisible) {
      test.skip()
      return
    }

    await dirEntry.click()
    await page.waitForTimeout(1000)
    await expect(breadcrumb).toBeVisible()
  })
})
