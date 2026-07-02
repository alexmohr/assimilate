// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { test, expect } from '@playwright/test';
import { login } from './helpers/auth';

test.describe('Login flow', () => {
  test('login page renders and redirects to dashboard on valid credentials', async ({ browser }) => {
    const context = await browser.newContext();
    const page = await context.newPage();

    await page.goto('/login');
    await expect(page.getByLabel('Username')).toBeVisible();
    await expect(page.getByLabel('Password')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible();

    await login(page, 'admin', 'admin');

    await expect(page).toHaveURL('/');
    await context.close();
  });
});

test.describe('Dashboard', () => {
  test('summary widgets are visible', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText('Online Clients').or(page.getByText('ONLINE CLIENTS')).first()).toBeVisible();
    await expect(page.getByText('Overdue').or(page.getByText('OVERDUE')).first()).toBeVisible();
    await expect(page.getByText('Last Backup').or(page.getByText('LAST BACKUP')).first()).toBeVisible();
    await expect(page.getByText('Total Storage').or(page.getByText('TOTAL STORAGE')).first()).toBeVisible();
  });

  test('Online Agents stat uses correct field names from API', async ({ request }) => {
    const resp = await request.get('/api/stats/summary');
    expect(resp.ok()).toBe(true);
    const body = (await resp.json()) as Record<string, unknown>;

    // These fields drove the 0/0 display bug - verify the API uses the correct names.
    expect(typeof body['online_agents']).toBe('number');
    expect(typeof body['total_agents']).toBe('number');
    expect(body['total_agents']).toBeGreaterThan(0);
  });

  test('dashboard shows repository health section', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    await expect(page.getByRole('heading', { name: 'REPOSITORY HEALTH' }).first()).toBeVisible();
    await expect(page.getByRole('main').getByText('db-server-01').first()).toBeVisible();
    await expect(page.getByRole('main').getByText('media-store-01').first()).toBeVisible();
    await expect(page.getByRole('main').getByText('web-server-01').first()).toBeVisible();
  });

  test('dashboard shows recent activity section', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    const activityHeading = page.getByRole('heading', { name: 'RECENT ACTIVITY' });
    await expect(activityHeading).toBeVisible();
    await expect(activityHeading.locator('..').getByText('db-server-01').first()).toBeVisible();
  });

  test('dashboard shows backup stats section', async ({ page }) => {
    await page.goto('/');

    await expect(page.getByRole('heading', { name: 'BACKUP STATS' })).toBeVisible();
    await expect(page.getByText('SUCCESS RATE')).toBeVisible();
  });

  test('dashboard shows next scheduled section', async ({ page }) => {
    await page.goto('/');

    await expect(page.getByRole('heading', { name: 'NEXT SCHEDULED' })).toBeVisible();
  });
});

test.describe('Navigation sidebar', () => {
  test('Clients link navigates to /clients', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Clients' }).click();
    await expect(page).toHaveURL(/\/clients/);
  });

  test('Repos link navigates to /repos', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Repos' }).click();
    await expect(page).toHaveURL(/\/repos/);
  });

  test('Schedules link navigates to /schedules', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Schedules' }).click();
    await expect(page).toHaveURL(/\/schedules/);
  });

  test('Activity link navigates to /activity', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Activity' }).click();
    await expect(page).toHaveURL(/\/activity/);
  });

  test('Dashboard link returns to root', async ({ page }) => {
    await page.goto('/clients');
    await page.getByRole('link', { name: 'Dashboard' }).click();
    await expect(page).toHaveURL('/');
  });
});
