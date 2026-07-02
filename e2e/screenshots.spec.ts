// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import * as path from 'path';
import { test } from '@playwright/test';
import { login } from './helpers/auth';

test.use({ viewport: { width: 1280, height: 800 } });

const screenshotsDir = path.resolve(__dirname, '../docs/assets/screenshots');

test('dashboard', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'dashboard.png'),
    fullPage: true,
  });
});

test('hosts', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/clients');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'hosts.png'),
    fullPage: true,
  });
});

test('host-detail', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/clients/web-01');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'host-detail.png'),
    fullPage: true,
  });
});

test('repositories', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/repos');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'repositories.png'),
    fullPage: true,
  });
});

test('schedules', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/schedules');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'schedules.png'),
    fullPage: true,
  });
});

test('schedule-detail', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/schedules/2');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'schedule-detail.png'),
    fullPage: true,
  });
});

test('users', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/users');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'users.png'),
    fullPage: true,
  });
});

test('tokens', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/tokens');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'tokens.png'),
    fullPage: true,
  });
});

test('system', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/system');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'system.png'),
    fullPage: true,
  });
});

test('archives', async ({ page }) => {
  await login(page, 'admin', 'admin');
  await page.goto('/repos/1');
  await page.waitForLoadState('networkidle');
  await page.screenshot({
    path: path.join(screenshotsDir, 'archives.png'),
    fullPage: true,
  });
});
