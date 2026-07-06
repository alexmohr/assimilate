// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { test as base, expect, type Page } from '@playwright/test'

export const adminRoutes = [
  '/system',
  '/admin/roles',
  '/admin/groups',
  '/audit-log',
  '/notifications',
] as const

export async function verifyRedirectFromAdminRoutes(
  page: Page,
  routes: readonly string[],
  timeout = 10_000,
): Promise<void> {
  for (const route of routes) {
    await page.goto(route, { waitUntil: 'commit' })
    await page.waitForURL((url) => !url.pathname.startsWith(route), { timeout })
    await expect(page).not.toHaveURL(/\/error/)
    await expect(page).toHaveURL(/\/$/)
  }
}

export async function loginAsAdmin(page: Page): Promise<void> {
  // Retry the full login flow up to 3 times to handle transient CI slowness.
  let lastErr: unknown
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      await page.goto('/login')
      await page.locator('input[type="text"], input[name="username"]').fill('admin')
      await page.locator('input[type="password"]').fill('admin')
      // Wait for the login API response before checking the URL, so a slow
      // server round-trip does not cause waitForURL to race.
      await Promise.all([
        page.waitForResponse(
          (resp) => resp.url().includes('/api/auth/login') && resp.status() === 200,
          { timeout: 60_000 },
        ),
        page.locator('button[type="submit"]').click(),
      ])
      // 'commit' resolves as soon as the response headers arrive, without
      // waiting for the full dashboard to load. This avoids a race where a
      // slow CI runner can't load all dashboard API responses within the
      // navigation timeout, even though the URL has already changed.
      await page.waitForURL((url) => !new URL(url).pathname.startsWith('/login'), {
        timeout: 60_000,
        waitUntil: 'commit',
      })
      return
    } catch (err) {
      lastErr = err
      if (attempt < 3) {
        await page.waitForTimeout(2_000)
      }
    }
  }
  throw lastErr
}

// Wraps the built-in `page` fixture to collect Istanbul coverage after each
// test when VITE_COVERAGE=true. The browser accumulates `window.__coverage__`
// throughout the test; we read it out just before Playwright closes the page
// and write a JSON file to `.nyc_output/` for later `nyc report` processing.
async function captureCoverage(page: Page): Promise<void> {
  if (process.env.VITE_COVERAGE !== 'true') return
  const coverage = await page
    .evaluate(() => (window as Window & { __coverage__?: object }).__coverage__ ?? null)
    .catch(() => null)
  if (!coverage) return
  const dir = join(process.cwd(), '.nyc_output')
  await mkdir(dir, { recursive: true })
  const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`
  await writeFile(join(dir, `e2e-${id}.json`), JSON.stringify(coverage))
}

export const test = base.extend<{ page: Page }>({
  page: async ({ page }, use) => {
    await use(page)
    await captureCoverage(page)
  },
})

export { expect }
