// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('storage trend widget renders on dashboard', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await expect(page.getByText('Storage Trend')).toBeVisible({ timeout: 10_000 })
})

test('storage trend widget shows deduplicated chart section', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  // Wait for the Storage Trend panel to appear
  await expect(page.getByText('Storage Trend')).toBeVisible({ timeout: 10_000 })
  // The deduplicated metric label should be present
  await expect(page.getByText('Deduplicated')).toBeVisible({ timeout: 10_000 })
})

test('storage trend widget shows original and compressed chart section', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await expect(page.getByText('Storage Trend')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('Original & Compressed')).toBeVisible({ timeout: 10_000 })
})

test('storage trend widget time range controls are functional', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await expect(page.getByText('Storage Trend')).toBeVisible({ timeout: 10_000 })

  // The time range toggle buttons should be visible
  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  await expect(trendPanel.getByText('30d')).toBeVisible()
  await expect(trendPanel.getByText('14d')).toBeVisible()
  await expect(trendPanel.getByText('90d')).toBeVisible()
  await expect(trendPanel.getByText('1y')).toBeVisible()

  // Clicking 90d should not cause an error
  await trendPanel.getByText('90d').click()
  await page.waitForTimeout(1_000)
  await expect(page).not.toHaveURL(/\/error/)
})

test('storage trend widget repo filter dropdown is present', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await expect(page.getByText('Storage Trend')).toBeVisible({ timeout: 10_000 })

  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  // The "All Repos" option should appear in the select dropdown
  await expect(trendPanel.getByText('All Repos')).toBeVisible()
})

test('storage trend charts render with demo data', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  // Allow time for API calls to complete and charts to render
  await page.waitForTimeout(3_000)
  await expect(page).not.toHaveURL(/\/error/)

  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  // Demo data has 30+ days of backups, so charts should render (not show "Not enough data")
  await expect(trendPanel.getByText('Not enough data.')).not.toBeVisible()
  // Chart containers should be present
  await expect(trendPanel.locator('.chart-container').first()).toBeVisible()
})
