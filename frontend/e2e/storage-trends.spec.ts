// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('storage trend widget renders on dashboard', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await expect(page.locator('h2', { hasText: 'Storage Trend' }).first()).toBeVisible({
    timeout: 10_000,
  })
})

test('storage trend widget shows deduplicated chart section', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  await expect(trendPanel).toBeVisible({ timeout: 10_000 })
  // The deduplicated metric label is only rendered when hasData (≥2 trend entries)
  await expect(trendPanel.locator('.metric-label', { hasText: 'Deduplicated' })).toBeVisible({
    timeout: 10_000,
  })
})

test('storage trend widget shows original and compressed chart section', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  await expect(trendPanel).toBeVisible({ timeout: 10_000 })
  await expect(trendPanel.locator('.metric-label', { hasText: 'Original' })).toBeVisible({
    timeout: 10_000,
  })
})

test('storage trend widget time range controls are functional', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  await expect(trendPanel).toBeVisible({ timeout: 10_000 })

  await expect(trendPanel.getByText('30d')).toBeVisible()
  await expect(trendPanel.getByText('14d')).toBeVisible()
  await expect(trendPanel.getByText('90d')).toBeVisible()
  await expect(trendPanel.getByText('1y')).toBeVisible()

  await trendPanel.getByText('90d').click()
  await page.waitForTimeout(1_000)
  await expect(page).not.toHaveURL(/\/error/)
})

test('storage trend widget repo filter dropdown is present', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  await expect(trendPanel).toBeVisible({ timeout: 10_000 })
  // The select element itself should be visible; it always has the "All Repos" option selected
  await expect(trendPanel.locator('select.stats-select')).toBeVisible()
})

test('storage trend charts render with demo data', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await page.waitForTimeout(3_000)
  await expect(page).not.toHaveURL(/\/error/)

  const trendPanel = page.locator('.panel').filter({ hasText: 'Storage Trend' })
  await expect(trendPanel.getByText('Not enough data.')).not.toBeVisible()
  await expect(trendPanel.locator('.chart-container').first()).toBeVisible()
})
