// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test';

test.describe('Hosts Management', () => {
  test('hosts list shows connected agent hosts', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('web-server-01')).toBeVisible();
    await expect(page.getByText('db-server-01')).toBeVisible();
    await expect(page.getByText('media-store-01')).toBeVisible();
  });

  test('hosts list shows imported/unmatched hosts', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('old-webserver')).toBeVisible();
    await expect(page.getByText('legacy-db-prod')).toBeVisible();
  });

  test('hosts list shows online/offline status indicators', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    const statusIndicators = page.locator('[class*="status"], [class*="online"], [class*="offline"], [data-status]');
    await expect(statusIndicators.first()).toBeVisible();
  });

  test('clicking a host navigates to detail page', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    await page.locator('.host-card').filter({ hasText: 'web-server-01' }).first().click();
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/clients\//);
  });

  test('host detail shows hostname and connection status', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    await page.locator('.host-card').filter({ hasText: 'web-server-01' }).first().click();
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('web-server-01')).toBeVisible();

    const connectionInfo = page.locator(
      '[class*="status"], [class*="connect"], [class*="online"], [class*="offline"]',
    );
    await expect(connectionInfo.first()).toBeVisible();
  });

  test('host detail shows display name', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    await page.locator('.host-card').filter({ hasText: 'web-server-01' }).first().click();
    await page.waitForLoadState('networkidle');

    await expect(page.locator('h1, h2, [class*="title"], [class*="name"], [class*="display"]').first()).toBeVisible();
  });
});
