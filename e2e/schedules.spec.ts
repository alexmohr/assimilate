// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test';

test.describe('Schedules Management', () => {
  test('schedules list shows heading', async ({ page }) => {
    await page.goto('/schedules');
    await page.waitForLoadState('networkidle');

    await expect(page.getByRole('heading', { name: 'Schedules' })).toBeVisible();
  });

  test('schedules list shows server-daily, database-hourly, and media-weekly', async ({ page }) => {
    await page.goto('/schedules');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('server-daily')).toBeVisible();
    await expect(page.getByText('database-hourly')).toBeVisible();
    await expect(page.getByText('media-weekly')).toBeVisible();
  });

  test('schedules list shows associated client hostnames', async ({ page }) => {
    await page.goto('/schedules');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('web-server-01')).toBeVisible();
    await expect(page.getByText('db-server-01')).toBeVisible();
    await expect(page.getByText('media-store-01')).toBeVisible();
  });

  test('clicking a schedule navigates to detail page', async ({ page }) => {
    await page.goto('/schedules');
    await page.waitForLoadState('networkidle');

    await page.getByText('server-daily').first().click();
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/schedules\/\d+/);
  });

  test('schedule detail shows cron expression', async ({ page }) => {
    await page.goto('/schedules/1');
    await page.waitForLoadState('networkidle');

    await expect(page.getByRole('textbox', { name: '0 2 * * *' })).toBeVisible();
  });

  test('schedule detail shows human-readable cron description', async ({ page }) => {
    await page.goto('/schedules/1');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('Daily at 02:00').first()).toBeVisible();
  });

  test('schedule detail shows retention policy', async ({ page }) => {
    await page.goto('/schedules/1');
    await page.waitForLoadState('networkidle');

    await expect(page.getByRole('heading', { name: 'RETENTION' })).toBeVisible();
    await expect(page.getByText('Daily', { exact: true })).toBeVisible();
    await expect(page.getByText('Weekly', { exact: true })).toBeVisible();
  });

  test('schedule detail shows host and repository assignment', async ({ page }) => {
    await page.goto('/schedules/1');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('Client', { exact: true })).toBeVisible();
    await expect(page.getByText('Repository', { exact: true })).toBeVisible();
    await expect(page.getByText('server-daily')).toBeVisible();
  });

  test('schedule detail results tab shows backup reports', async ({ page }) => {
    await page.goto('/schedules/1');
    await page.waitForLoadState('networkidle');

    await page.getByRole('button', { name: 'Results' }).click();
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('SUCCESS').first()).toBeVisible();
  });

  test('schedule detail results tab shows varied statuses', async ({ page }) => {
    await page.goto('/schedules/1');
    await page.waitForLoadState('networkidle');

    await page.getByRole('button', { name: 'Results' }).click();
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('SUCCESS').first()).toBeVisible();
    await expect(page.getByText('WARNING').first()).toBeVisible();
    await expect(page.getByText('FAILED').first()).toBeVisible();
  });
});
