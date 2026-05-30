// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test';

test.describe('Admin Journey', () => {
  test.describe('Users', () => {
    test('users list shows all seeded users', async ({ page }) => {
      await page.goto('/users');
      await page.waitForLoadState('networkidle');

      await expect(page.getByText('operator1')).toBeVisible();
      await expect(page.getByText('viewer1')).toBeVisible();
    });

    test('users have roles assigned', async ({ page }) => {
      await page.goto('/users');
      await page.waitForLoadState('networkidle');

      await expect(page.getByText('admin').first()).toBeVisible();
      await expect(page.getByText('operator').first()).toBeVisible();
      await expect(page.getByText('viewer').first()).toBeVisible();
    });
  });

  test.describe('Groups', () => {
    test('groups page shows seeded groups', async ({ page }) => {
      await page.goto('/admin/groups');
      await page.waitForLoadState('networkidle');

      await expect(page.getByText('backend-team')).toBeVisible();
      await expect(page.getByText('data-team')).toBeVisible();
    });
  });

  test.describe('Audit Log', () => {
    test('audit log page loads', async ({ page }) => {
      await page.goto('/audit-log');
      await page.waitForLoadState('networkidle');

      await expect(page).toHaveURL(/\/audit-log/);
    });

    test('audit log shows events', async ({ page }) => {
      await page.goto('/audit-log');
      await page.waitForLoadState('networkidle');

      const eventRows = page.locator('.audit-table tbody tr, .audit-log tr, table tbody tr');
      await expect(eventRows.first()).toBeVisible();
    });

    test('audit log contains repo creation or login events', async ({ page }) => {
      await page.goto('/audit-log');
      await page.waitForLoadState('networkidle');

      const badges = page.locator('.badge');
      await expect(badges.first()).toBeVisible();

      const badgeText = await badges.allInnerTexts();
      const hasExpectedAction = badgeText.some(
        (t) =>
          t.toLowerCase().includes('create') ||
          t.toLowerCase().includes('login') ||
          t.toLowerCase().includes('update'),
      );
      expect(hasExpectedAction).toBe(true);
    });
  });
});
