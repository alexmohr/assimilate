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

// Patches schedule 1 ("server-daily", targeting web-server-01) in both
// /api/schedules and /api/schedules/:id responses, so tests can force
// scheduler-driven fields (e.g. the auto-disable bookkeeping) without waiting
// out real backoff ticks. Used by every surface that reads schedule 1 through
// these endpoints: the schedules list, its detail page, and the agent-detail
// schedules tab.
export async function mockScheduleOnePatch(
  page: Page,
  overrides: Record<string, unknown>,
): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/schedules' || /^\/api\/schedules\/\d+$/.test(url.pathname),
    async (route) => {
      const response = await route.fetch()
      const body = (await response.json()) as
        | Array<Record<string, unknown>>
        | Record<string, unknown>
      const patched = Array.isArray(body)
        ? body.map((s) => (s.id === 1 ? { ...s, ...overrides } : s))
        : body.id === 1
          ? { ...body, ...overrides }
          : body
      return route.fulfill({
        status: response.status(),
        contentType: 'application/json',
        body: JSON.stringify(patched),
      })
    },
  )
}

// Injects a running backup operation for web-server-01 / server-daily (schedule 1,
// seeded by the demo) into /api/stats/dashboard-overview's running_operations, so
// tests can force the agent list's "Running" pill to render without waiting for a
// real backup to start.
export async function mockRunningBackupOperation(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/stats/dashboard-overview',
    async (route) => {
      const response = await route.fetch()
      const overview = (await response.json()) as { running_operations: unknown[] }
      overview.running_operations = [
        ...overview.running_operations,
        {
          report_id: 999_999,
          status: 'started',
          hostname: 'web-server-01',
          schedule_id: 1,
          schedule_name: 'server-daily',
          repo_id: 1,
          repo_name: 'server-daily',
          started_at: new Date().toISOString(),
          destination: { kind: 'schedule', schedule_id: 1 },
        },
      ]
      return route.fulfill({
        status: response.status(),
        contentType: 'application/json',
        body: JSON.stringify(overview),
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
  // The loading spinner shares the .state-msg--inline class with the genuine
  // terminal empty/error states, so waiting for "any of archive-group /
  // archive-row-detailed / state-msg--inline" can resolve on the transient
  // spinner before the archive groups actually render, leaving them
  // collapsed for the rest of the test. Wait out the spinner explicitly
  // first.
  await page
    .locator('.state-msg--inline .spinner-icon')
    .waitFor({ state: 'hidden', timeout: 20_000 })
    .catch(() => {})

  await page
    .locator('.archive-group, .archive-row-detailed, .state-msg--inline')
    .first()
    .waitFor({ state: 'visible', timeout: 20_000 })
    .catch(() => {})
  // Click the chevron, not the header button itself: the header also
  // contains a BaseHostLink that fills most of its width, and clicking the
  // button's center can land on that link (navigating away) instead of
  // toggling the group.
  const collapsedChevrons = page.locator('.group-header.collapsed .group-chevron')
  while ((await collapsedChevrons.count()) > 0) {
    await collapsedChevrons.first().click()
  }
}
