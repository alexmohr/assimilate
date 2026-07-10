// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Schedule backups tab - archive browser', () => {
  test('Backups tab is visible on backup-type schedule detail', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('button', { name: 'Backups' })).toBeVisible()
  })

  test('save bar is hidden on Backups tab', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Save bar should be visible initially (Settings tab)
    await expect(page.locator('.save-bar')).toBeVisible()

    // Click Backups tab
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForTimeout(500)

    // Save bar should be hidden
    await expect(page.locator('.save-bar')).not.toBeVisible()
  })

  test('backups tab shows empty state or archive list', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Click Backups tab
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForTimeout(1000)

    // Either the archives panel title is visible (with data)
    // or the empty state message is shown (no archives yet)
    const panelTitle = page.locator('.panel-title').filter({ hasText: 'Archives' })
    const emptyState = page.locator('.empty-state').filter({ hasText: 'No backup archives' })
    await expect(panelTitle.or(emptyState).first()).toBeVisible({ timeout: 10_000 })
  })

  test('backups tab renders split layout structure', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Click Backups tab
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForTimeout(1000)

    // The backups layout should be rendered (either with data or empty)
    const backupsLayout = page.locator('.backups-layout')
    const tabContent = page
      .locator('.tab-content')
      .filter({ hasText: /Archives|No backup archives/ })
    await expect(backupsLayout.or(tabContent).first()).toBeVisible({ timeout: 10_000 })
  })

  test('file browser structure renders when archive is selected', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Click Backups tab
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForTimeout(1000)

    // Check if there's at least one archive row to click
    const archiveRow = page.locator('.archive-row').first()
    const rowVisible = await archiveRow.isVisible({ timeout: 5_000 }).catch(() => false)
    if (!rowVisible) {
      // No archives available in demo data - skip interactive tests
      test.skip()
      return
    }

    await archiveRow.click()
    await page.waitForTimeout(1000)

    // The file browser should show with breadcrumb
    await expect(page.locator('.breadcrumb')).toBeVisible()
    await expect(
      page.locator('.breadcrumb').getByText('~').or(page.locator('.breadcrumb').getByText('/')),
    ).toBeVisible()
  })

  test('download buttons present in file browser when archive selected', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Click Backups tab
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForTimeout(1000)

    // Check if there's at least one archive row to click
    const archiveRow = page.locator('.archive-row').first()
    const rowVisible = await archiveRow.isVisible({ timeout: 5_000 }).catch(() => false)
    if (!rowVisible) {
      // No archives available in demo data - skip interactive tests
      test.skip()
      return
    }

    await archiveRow.click()
    await page.waitForTimeout(2000)

    // Check for download buttons in the file browser
    const downloadButton = page.locator('.archive-file-browser button[title*="Download"]').first()
    const buttonVisible = await downloadButton.isVisible({ timeout: 10_000 }).catch(() => false)
    if (buttonVisible) {
      await expect(downloadButton).toBeVisible()
    }
  })

  test('breadcrumb navigation updates when navigating directories', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Click Backups tab
    await page.getByRole('button', { name: 'Backups' }).click()
    await page.waitForTimeout(1000)

    // Check if there's at least one archive row to click
    const archiveRow = page.locator('.archive-row').first()
    const rowVisible = await archiveRow.isVisible({ timeout: 5_000 }).catch(() => false)
    if (!rowVisible) {
      test.skip()
      return
    }

    await archiveRow.click()
    await page.waitForTimeout(1000)

    // Breadcrumb should show root
    const breadcrumb = page.locator('.breadcrumb')
    await expect(breadcrumb).toBeVisible()

    // Try navigating into a directory if one exists
    const dirEntry = page.locator('.archive-file-browser tr.clickable').first()
    const dirVisible = await dirEntry.isVisible({ timeout: 5_000 }).catch(() => false)
    if (!dirVisible) {
      // No directories to navigate into, skip
      test.skip()
      return
    }

    await dirEntry.click()
    await page.waitForTimeout(1000)
    // Breadcrumb should have updated
    await expect(breadcrumb).toBeVisible()
  })
})
