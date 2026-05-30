// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import type { Browser, Page } from '@playwright/test';

const ADMIN_STORAGE_STATE_PATH = resolve('.auth/admin.json');

export async function login(
  page: Page,
  username: string,
  password: string,
): Promise<void> {
  await page.goto('/login');
  await page.getByLabel('Username').fill(username);
  await page.getByLabel('Password').fill(password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.waitForURL((url) => !url.pathname.startsWith('/login'));
}

export async function loginAsAdmin(page: Page): Promise<void> {
  await login(page, 'admin', 'admin');
}

export async function loginAsOperator(page: Page): Promise<void> {
  await login(page, 'operator1', 'operator1');
}

export async function loginAsViewer(page: Page): Promise<void> {
  await login(page, 'viewer1', 'viewer1');
}

export async function getAdminStorageState(browser: Browser): Promise<string> {
  await mkdir(dirname(ADMIN_STORAGE_STATE_PATH), { recursive: true });
  const context = await browser.newContext();
  const page = await context.newPage();

  await loginAsAdmin(page);
  await context.storageState({ path: ADMIN_STORAGE_STATE_PATH });

  await context.close();
  return ADMIN_STORAGE_STATE_PATH;
}
