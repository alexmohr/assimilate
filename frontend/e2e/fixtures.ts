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

async function login(page: Page, username: string, password: string): Promise<void> {
  // Retry the full login flow up to 3 times to handle transient CI slowness.
  let lastErr: unknown
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      await page.goto('/login')
      await page.locator('input[type="text"], input[name="username"]').fill(username)
      await page.locator('input[type="password"]').fill(password)
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

export async function loginAsAdmin(page: Page): Promise<void> {
  await login(page, 'admin', 'admin')
}

export async function loginAsOperator(page: Page): Promise<void> {
  await login(page, 'operator1', 'operator1')
}

export async function loginAsViewer(page: Page): Promise<void> {
  await login(page, 'viewer1', 'viewer1')
}

// Overrides the web-server-01 / server-daily health entry (schedule 1, seeded
// by the demo) for /api/stats/health so tests can force a specific chip
// (Overdue/Failed) to render without relying on the demo's seeded health
// state. Used by both the schedules list and the agent-detail schedules tab,
// which read the same schedule via the same endpoint.
export async function mockScheduleOneHealth(
  page: Page,
  overrides: Record<string, unknown>,
): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/stats/health',
    async (route) => {
      const response = await route.fetch()
      const entries = (await response.json()) as Array<Record<string, unknown>>
      const withoutTarget = entries.filter(
        (e) => !(e.schedule_id === 1 && e.hostname === 'web-server-01'),
      )
      withoutTarget.push({
        schedule_id: 1,
        hostname: 'web-server-01',
        target_name: 'server-daily',
        last_status: 'success',
        last_backup_at: '2020-01-01T02:00:00Z',
        is_overdue: false,
        last_error_message: null,
        cron_expression: '0 2 * * *',
        schedule_enabled: true,
        ...overrides,
      })
      return route.fulfill({
        status: response.status(),
        contentType: 'application/json',
        body: JSON.stringify(withoutTarget),
      })
    },
  )
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

// Archive host groups start collapsed by default, so .archive-row elements
// are hidden until their group is expanded. Wait for the list to settle into
// some terminal state first, since callers vary in how much they've already
// waited for the archives fetch to resolve.
export async function expandAllArchiveGroups(page: Page): Promise<void> {
  await page
    .locator('.archive-group, .archive-row-detailed, .state-msg-sm')
    .first()
    .waitFor({ state: 'visible', timeout: 20_000 })
    .catch(() => {})
  const collapsed = page.locator('.group-header.collapsed')
  while ((await collapsed.count()) > 0) {
    await collapsed.first().click()
  }
}
