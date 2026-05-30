// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test';

test.describe('Archives Browsing & Diff Journey', () => {
  test('archives tab loads showing archive entries with names and dates', async ({ page }) => {
    await page.goto('/repos/1?tab=archives');
    await page.waitForLoadState('networkidle');

    await expect(page.getByRole('button', { name: 'Archives' })).toBeVisible();
    await expect(page.locator('.panel-title').filter({ hasText: 'Archives' })).toBeVisible();

    await expect(page.getByRole('columnheader', { name: 'Name' })).toBeVisible();
    await expect(page.getByRole('columnheader', { name: 'Date' })).toBeVisible();
    await expect(page.getByRole('columnheader', { name: 'Host' })).toBeVisible();
  });

  test('archive list contains web-server-01 backup entries', async ({ page }) => {
    await page.goto('/repos/1?tab=archives');
    await page.waitForLoadState('networkidle');

    await expect(page.getByText(/web-server-01-backup/).first()).toBeVisible();
  });

  test('clicking an archive shows file tree browser', async ({ page }) => {
    await page.goto('/repos/1?tab=archives');
    await page.waitForLoadState('networkidle');

    await page.getByText(/web-server-01-backup/).first().click();
    await page.waitForTimeout(1000);

    await expect(page.locator('.panel-title').filter({ hasText: /Files/ })).toBeVisible();
    await expect(page.locator('.archive-breadcrumb')).toBeVisible();

    const browserPanel = page.locator('.browser-panel').last();
    await expect(browserPanel).toBeVisible();
  });

  test('file browser shows directory entries with names and modified dates', async ({ page }) => {
    await page.goto('/repos/1?tab=archives');
    await page.waitForLoadState('networkidle');

    await page.getByText(/web-server-01-backup/).first().click();
    await page.waitForTimeout(1000);

    const browserPanel = page.locator('.browser-panel').last();
    await expect(browserPanel.getByText('Name')).toBeVisible();
    await expect(browserPanel.getByText('Modified')).toBeVisible();
    await expect(browserPanel.getByText('etc')).toBeVisible();
  });

  test('file browser breadcrumb shows root path', async ({ page }) => {
    await page.goto('/repos/1?tab=archives');
    await page.waitForLoadState('networkidle');

    await page.getByText(/web-server-01-backup/).first().click();
    await page.waitForTimeout(1000);

    await expect(page.locator('.archive-breadcrumb').getByText('~')).toBeVisible();
  });

  test('clicking a directory in file browser navigates into it', async ({ page }) => {
    await page.goto('/repos/1?tab=archives');
    await page.waitForLoadState('networkidle');

    await page.getByText(/web-server-01-backup/).first().click();
    await page.waitForTimeout(1000);

    const browserPanel = page.locator('.browser-panel').last();
    await browserPanel.getByText('etc').click();
    await page.waitForTimeout(1000);

    await expect(page.locator('.archive-breadcrumb')).toContainText('etc');
  });

  test('archive tags API endpoint is accessible and returns structured data', async ({
    request,
  }) => {
    const archivesRes = await request.get('/api/repos/1/archives');
    expect(archivesRes.ok()).toBeTruthy();

    const archives: { name: string }[] = await archivesRes.json();
    expect(archives.length).toBeGreaterThan(0);

    const tagsRes = await request.get(
      `/api/repos/1/archives/${encodeURIComponent(archives[0].name)}/tags`,
    );
    expect(tagsRes.ok()).toBeTruthy();
    const tags: unknown = await tagsRes.json();
    expect(Array.isArray(tags)).toBeTruthy();
  });

  test('pre-upgrade and weekly-baseline archive tags exist in demo data', async ({ request }) => {
    const archivesRes = await request.get('/api/repos/1/archives');
    expect(archivesRes.ok()).toBeTruthy();
    const archives: { name: string }[] = await archivesRes.json();
    expect(archives.length).toBeGreaterThan(0);

    const basePattern = archives[0].name.replace(/T\d{2}:\d{2}:\d{2}$/, 'T');
    const secondsVariants = Array.from({ length: 120 }, (_, i) => {
      const s = String(i).padStart(2, '0');
      return `${basePattern}23:12:${s}`;
    });

    let foundPreUpgrade = false;
    let foundWeeklyBaseline = false;

    for (const name of secondsVariants) {
      if (foundPreUpgrade && foundWeeklyBaseline) break;
      const res = await request.get(
        `/api/repos/1/archives/${encodeURIComponent(name)}/tags`,
      );
      if (!res.ok()) continue;
      const tags: { tag: string }[] = await res.json();
      for (const t of tags) {
        if (t.tag === 'pre-upgrade') foundPreUpgrade = true;
      }
    }

    const webServerArchives = archives.filter((a) => a.name.startsWith('web-server-01-backup-'));
    const oldestWebServerArchive = webServerArchives[webServerArchives.length - 1];
    const base3DayPattern = oldestWebServerArchive?.name.replace(/T\d{2}:\d{2}:\d{2}$/, 'T');

    if (base3DayPattern) {
      for (let s = 0; s < 120 && !foundWeeklyBaseline; s++) {
        const sec = String(s).padStart(2, '0');
        const name = `${base3DayPattern}23:12:${sec}`;
        const res = await request.get(
          `/api/repos/1/archives/${encodeURIComponent(name)}/tags`,
        );
        if (!res.ok()) continue;
        const tags: { tag: string }[] = await res.json();
        for (const t of tags) {
          if (t.tag === 'weekly-baseline') foundWeeklyBaseline = true;
        }
      }
    }

    expect(foundPreUpgrade, 'pre-upgrade archive tag should exist in demo data').toBeTruthy();
    expect(
      foundWeeklyBaseline,
      'weekly-baseline archive tag should exist in demo data',
    ).toBeTruthy();
  });

  test('archive diff API returns structured results for two archives', async ({ request }) => {
    const archivesRes = await request.get('/api/repos/1/archives');
    expect(archivesRes.ok()).toBeTruthy();

    const archives: { name: string }[] = await archivesRes.json();
    expect(archives.length).toBeGreaterThanOrEqual(2);

    const [first, second] = archives;
    const diffRes = await request.get(
      `/api/repos/1/archives/diff?archive1=${encodeURIComponent(first.name)}&archive2=${encodeURIComponent(second.name)}`,
    );
    expect(diffRes.ok()).toBeTruthy();

    const diff: unknown = await diffRes.json();
    expect(diff).toBeDefined();
  });

  test('navigating to /archives redirects to repositories list', async ({ page }) => {
    await page.goto('/archives');
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/\/repos/);
  });

  test('archives tab is accessible from repository detail overview tab', async ({ page }) => {
    await page.goto('/repos/1');
    await page.waitForLoadState('networkidle');

    const archivesTab = page.getByRole('button', { name: 'Archives' });
    await expect(archivesTab).toBeVisible();

    await archivesTab.click();
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveURL(/tab=archives/);
    await expect(page.locator('.panel-title').filter({ hasText: 'Archives' })).toBeVisible();
  });
});
