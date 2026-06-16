// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test';

interface ScheduleListEntry {
  id: number;
  name: string;
  target_hostnames: string[];
}

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

  test('schedule detail with per-host backup sources loads without error', async ({
    request,
    page,
  }) => {
    // Find the multi-agent schedule seeded with backup_sources_per_agent.
    const listResp = await request.get('/api/schedules');
    expect(listResp.ok()).toBe(true);
    const schedules = (await listResp.json()) as ScheduleListEntry[];

    const multiHost = schedules.find(
      (s) =>
        s.target_hostnames.includes('web-server-01') &&
        s.target_hostnames.includes('db-server-01') &&
        s.target_hostnames.includes('media-store-01'),
    );
    expect(multiHost).toBeDefined();

    // Navigate to the detail page — this used to crash before the null-safety fix.
    await page.goto(`/schedules/${multiHost!.id}`);
    await page.waitForLoadState('networkidle');

    // Page must stay on the schedule detail URL (no redirect or error page).
    await expect(page).toHaveURL(`/schedules/${multiHost!.id}`);

    // Per-host backup sources section should be rendered.
    await expect(page.locator('.per-host-paths').or(page.locator('.per-host-entry')).first()).toBeVisible();
  });

  test('creating a new schedule succeeds (regression: agent_ids/_per_agent field naming)', async ({
    page,
  }) => {
    await page.goto('/schedules/new');
    await page.waitForLoadState('networkidle');

    const targetCard = page.locator('.form-card', { hasText: 'Target' });

    await targetCard.locator('.multi-select-trigger').click();
    await targetCard.getByText('Production Web Server').click();

    // Close the dropdown so it doesn't cover the repository select.
    await page.getByPlaceholder('e.g. Daily web server backup').click();

    await targetCard
      .locator('.form-group', { hasText: 'Repository' })
      .locator('select')
      .selectOption({ label: 'server-daily' });

    // Use Integrity Check so the test doesn't depend on backup source paths.
    await targetCard
      .locator('.form-group', { hasText: 'Schedule Type' })
      .locator('select')
      .selectOption({ label: 'Integrity Check' });

    await page.getByRole('button', { name: 'Create Schedule' }).click();

    // The create request used to fail with "missing field `agent_ids`" because the
    // frontend sent client_ids/backup_sources_per_host instead of the names the
    // backend expects. A successful save navigates to the new schedule's detail page.
    await expect(page).toHaveURL(/\/schedules\/\d+$/);
    await expect(page.locator('.error-inline')).not.toBeVisible();
  });
});
