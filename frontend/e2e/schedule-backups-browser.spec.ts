// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

// All tests run against the demo environment seeded by seed-demo.sh.
// No API mocking — every request hits the real server.
//
// The seeded schedule used here is the first one on the list:
// web-server-01 → server-daily repo, 30+ days of successful backups.

async function goToBackupsTab(page: Page): Promise<void> {
  await loginAsAdmin(page)
  await page.goto('/schedules')
  await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })
  await page.locator('.schedule-card').first().click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 10_000 })
  await page.locator('.tab-bar').waitFor({ timeout: 10_000 })
  await page.getByRole('button', { name: 'Backups' }).click()
  await page.waitForURL(/tab=backups/)
}

test('Backups tab is visible on a backup-type schedule', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/schedules')
  await page.locator('.schedule-card').first().waitFor({ timeout: 10_000 })
  await page.locator('.schedule-card').first().click()
  await page.waitForURL(/\/schedules\/\d+/, { timeout: 10_000 })

  await expect(page.getByRole('button', { name: 'Backups' })).toBeVisible({ timeout: 10_000 })
})

test('Backups tab shows the archive list panel with seeded archives', async ({ page }) => {
  await goToBackupsTab(page)

  await expect(page.locator('.archive-list-panel')).toBeVisible({ timeout: 10_000 })
  await expect(page.locator('.archive-row').first()).toBeVisible({ timeout: 10_000 })
})

test('archive rows show the agent hostname', async ({ page }) => {
  await goToBackupsTab(page)

  await expect(page.locator('.archive-list-panel')).toBeVisible({ timeout: 10_000 })
  // Seeded archives are from web-server-01.
  await expect(page.locator('.archive-list-panel')).toContainText('web-server-01', {
    timeout: 10_000,
  })
})

test('clicking an archive row selects it and opens the file browser panel', async ({ page }) => {
  await goToBackupsTab(page)

  await page.locator('.archive-row').first().waitFor({ timeout: 10_000 })
  await page.locator('.archive-row').first().click()

  await expect(page.locator('.archive-row-selected')).toBeVisible({ timeout: 5_000 })
  await expect(page.locator('.archive-browser-panel')).toBeVisible({ timeout: 5_000 })
  await expect(page.locator('.archive-browser-title')).not.toBeEmpty()
})

test('file browser loads and shows entries or indexing state — never an error page', async ({
  page,
}) => {
  await goToBackupsTab(page)

  await page.locator('.archive-row').first().waitFor({ timeout: 10_000 })
  await page.locator('.archive-row').first().click()
  await expect(page.locator('.archive-browser-panel')).toBeVisible({ timeout: 5_000 })

  // The first access triggers archive indexing, which may take a few seconds.
  await page.waitForTimeout(5_000)
  await expect(page).not.toHaveURL(/\/error/)

  const hasEntries = await page.locator('.archive-browser-panel .archive-dir-row').isVisible()
  const hasSpinner = await page.locator('.archive-browser-panel .archive-state').isVisible()
  expect(hasEntries || hasSpinner).toBe(true)
})

test('navigating into a directory updates the breadcrumb', async ({ page }) => {
  await goToBackupsTab(page)

  await page.locator('.archive-row').first().waitFor({ timeout: 10_000 })
  await page.locator('.archive-row').first().click()
  await expect(page.locator('.archive-browser-panel')).toBeVisible({ timeout: 5_000 })

  // Wait for indexing to complete and a directory row to appear.
  const dirRow = page.locator('.archive-dir-row').first()
  await expect(dirRow).toBeVisible({ timeout: 20_000 })
  const dirName = (await dirRow.locator('.cell-name').textContent())?.trim() ?? ''

  await dirRow.click()

  await expect(page.locator('.archive-breadcrumb')).toContainText(dirName, { timeout: 5_000 })
})

test('clicking the root breadcrumb crumb returns to the archive root', async ({ page }) => {
  await goToBackupsTab(page)

  await page.locator('.archive-row').first().waitFor({ timeout: 10_000 })
  await page.locator('.archive-row').first().click()
  await expect(page.locator('.archive-browser-panel')).toBeVisible({ timeout: 5_000 })

  const dirRow = page.locator('.archive-dir-row').first()
  await expect(dirRow).toBeVisible({ timeout: 20_000 })
  const dirName = (await dirRow.locator('.cell-name').textContent())?.trim() ?? ''
  await dirRow.click()
  await expect(page.locator('.archive-breadcrumb')).toContainText(dirName, { timeout: 5_000 })

  // Click the first (root) crumb to navigate back.
  await page.locator('.archive-crumb').first().click()

  await expect(page.locator('.archive-breadcrumb')).not.toContainText(dirName, { timeout: 5_000 })
})

test('download button is present for each entry in the file browser', async ({ page }) => {
  await goToBackupsTab(page)

  await page.locator('.archive-row').first().waitFor({ timeout: 10_000 })
  await page.locator('.archive-row').first().click()
  await expect(page.locator('.archive-browser-panel')).toBeVisible({ timeout: 5_000 })

  // Wait for at least one entry row to appear.
  await expect(page.locator('.archive-browser-panel .report-row').first()).toBeVisible({
    timeout: 20_000,
  })

  await expect(page.locator('.cell-action button').first()).toBeVisible()
})

test('save bar is hidden on the Backups tab', async ({ page }) => {
  await goToBackupsTab(page)

  await expect(page.locator('.save-bar')).not.toBeVisible()
})
