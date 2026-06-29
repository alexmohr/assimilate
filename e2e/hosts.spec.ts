// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test, type Route } from '@playwright/test';

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

  test('deploy dialog opens and shows Load from remote button', async ({ page }) => {
    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    // Demo agents have no agent_version so they show a Deploy button (not imported).
    const deployBtn = page
      .locator('.host-card')
      .filter({ hasText: 'web-server-01' })
      .locator('.card-actions button', { hasText: /Deploy|Upgrade/ })
      .first();
    await expect(deployBtn).toBeVisible();
    await deployBtn.click();

    // Dialog should open.
    await expect(page.getByRole('heading', { name: /Deploy|Upgrade/ }).first()).toBeVisible();

    // The "Load from remote" button must be present — this was added in issue #124.
    const loadBtn = page.getByRole('button', { name: 'Load from remote' });
    await expect(loadBtn).toBeVisible();
    await expect(loadBtn).not.toBeDisabled();
  });

  test('agent card shows expandable CardError for failed backups', async ({ page }) => {
    // Intercept the health API to inject a failure with an error message for web-server-01.
    await page.route('**/api/stats/health', async (route: Route) => {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify([
          {
            hostname: 'web-server-01',
            target_name: 'server-daily',
            last_status: 'failed',
            last_backup_at: new Date().toISOString(),
            is_overdue: false,
            last_error_message: 'Repository lock could not be acquired',
          },
        ]),
      });
    });

    await page.goto('/clients');
    await page.waitForLoadState('networkidle');

    const card = page.locator('.host-card').filter({ hasText: 'web-server-01' }).first();

    // The CardError toggle button should be visible and show the issue label.
    const errorToggle = card.locator('.error-toggle');
    await expect(errorToggle).toBeVisible();

    // Error message must be hidden initially.
    const errorPre = card.locator('.error-pre');
    await expect(errorPre).not.toBeVisible();

    // Click to expand — the error detail becomes visible.
    await errorToggle.click();
    await expect(errorPre).toBeVisible();
    await expect(errorPre).toContainText('Repository lock could not be acquired');

    // Click again to collapse.
    await errorToggle.click();
    await expect(errorPre).not.toBeVisible();
  });
});
