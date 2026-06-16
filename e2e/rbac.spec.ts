// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { mkdir } from 'node:fs/promises';
import { expect, test, type Browser, type Page } from '@playwright/test';
import { loginAsOperator, loginAsViewer } from './helpers/auth';

const OPERATOR_STATE = '.auth/operator.json';

async function createViewerPage(
  browser: Browser,
  baseURL: string | undefined,
): Promise<{ context: Awaited<ReturnType<Browser['newContext']>>; page: Page }> {
  const context = await browser.newContext({ baseURL });
  const page = await context.newPage();
  await loginAsViewer(page);

  return { context, page };
}

test.describe('RBAC - Operator permissions', () => {
  test.beforeAll(async ({ browser, baseURL }) => {
    await mkdir('.auth', { recursive: true });
    const context = await browser.newContext({ baseURL });
    const page = await context.newPage();
    await loginAsOperator(page);
    await context.storageState({ path: OPERATOR_STATE });
    await context.close();
  });

  test('operator can view hosts', async ({ browser, baseURL }) => {
    const context = await browser.newContext({ baseURL, storageState: OPERATOR_STATE });
    const page = await context.newPage();

    await page.goto('/agents');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/agents/);
    await expect(page).not.toHaveURL(/\/login/);
    await context.close();
  });

  test('operator can view repositories', async ({ browser, baseURL }) => {
    const context = await browser.newContext({ baseURL, storageState: OPERATOR_STATE });
    const page = await context.newPage();

    await page.goto('/repos');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/repos/);
    await context.close();
  });

  test('operator can view schedules', async ({ browser, baseURL }) => {
    const context = await browser.newContext({ baseURL, storageState: OPERATOR_STATE });
    const page = await context.newPage();

    await page.goto('/schedules');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/schedules/);
    await context.close();
  });

  test('operator cannot access admin users page', async ({ browser, baseURL }) => {
    const context = await browser.newContext({ baseURL, storageState: OPERATOR_STATE });
    const page = await context.newPage();

    await page.goto('/admin/users');
    await page.waitForLoadState('networkidle');

    const url = page.url();
    const isForbidden =
      !url.includes('/admin/users') ||
      (await page.locator('text=/forbidden|403|not allowed|access denied/i').count()) > 0 ||
      (await page.locator('[class*="forbidden"], [class*="unauthorized"], [class*="error"]').count()) > 0;
    expect(isForbidden).toBe(true);
    await context.close();
  });

  test('operator cannot access admin roles page', async ({ browser, baseURL }) => {
    const context = await browser.newContext({ baseURL, storageState: OPERATOR_STATE });
    const page = await context.newPage();

    await page.goto('/admin/roles');
    await page.waitForLoadState('networkidle');

    const url = page.url();
    const isForbidden =
      !url.includes('/admin/roles') ||
      (await page.locator('text=/forbidden|403|not allowed|access denied/i').count()) > 0 ||
      (await page.locator('[class*="forbidden"], [class*="unauthorized"], [class*="error"]').count()) > 0;
    expect(isForbidden).toBe(true);
    await context.close();
  });

  test('operator cannot access admin groups page', async ({ browser, baseURL }) => {
    const context = await browser.newContext({ baseURL, storageState: OPERATOR_STATE });
    const page = await context.newPage();

    await page.goto('/admin/groups');
    await page.waitForLoadState('networkidle');

    const url = page.url();
    const isForbidden =
      !url.includes('/admin/groups') ||
      (await page.locator('text=/forbidden|403|not allowed|access denied/i').count()) > 0 ||
      (await page.locator('[class*="forbidden"], [class*="unauthorized"], [class*="error"]').count()) > 0;
    expect(isForbidden).toBe(true);
    await context.close();
  });
});

test.describe('RBAC - Viewer permissions', () => {
  test('viewer can view dashboard', async ({ browser, baseURL }) => {
    const { context, page } = await createViewerPage(browser, baseURL);

    await page.goto('/');
    await page.waitForLoadState('networkidle');

    await expect(page).not.toHaveURL(/\/login/);
    await context.close();
  });

  test('viewer can view hosts list', async ({ browser, baseURL }) => {
    const { context, page } = await createViewerPage(browser, baseURL);

    await page.goto('/agents');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/agents/);
    await expect(page).not.toHaveURL(/\/login/);
    await context.close();
  });

  test('viewer does not see create/add buttons on hosts page', async ({ browser, baseURL }) => {
    const { context, page } = await createViewerPage(browser, baseURL);

    await page.goto('/agents');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/agents/);
    await context.close();
  });

  test('viewer does not see create/add buttons on repositories page', async ({ browser, baseURL }) => {
    const { context, page } = await createViewerPage(browser, baseURL);

    await page.goto('/repos');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/repos/);
    await context.close();
  });

  test('viewer does not see create/add buttons on schedules page', async ({ browser, baseURL }) => {
    const { context, page } = await createViewerPage(browser, baseURL);

    await page.goto('/schedules');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/schedules/);
    await context.close();
  });
});
