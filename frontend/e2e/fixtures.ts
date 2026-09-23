// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { test as base, expect, type Locator, type Page } from '@playwright/test'

/**
 * Minimal failed backup report satisfying the agent/schedule reports
 * endpoints' shared shape, for guaranteeing a "clean up failed backups" menu
 * item's visibility independent of whatever the demo seed's own failure
 * window currently contains. `scheduleId` is null for an agent-only test,
 * or a schedule's id when the report needs to belong to one.
 */
export function makeFailedReport(id: number, scheduleId: number | null = null): object {
  return {
    id,
    agent_id: 1,
    repo_id: 1,
    schedule_id: scheduleId,
    status: 'failed',
    started_at: new Date(Date.now() - 3600_000).toISOString(),
    finished_at: new Date().toISOString(),
    original_size: 0,
    compressed_size: 0,
    deduplicated_size: 0,
    files_processed: 0,
    duration_secs: 5,
    error_message: 'connection refused',
    warnings: [],
    borg_version: null,
    archive_name: null,
    borg_command: null,
    hostname: 'web-server-01',
    repo_name: 'server-daily',
    schedule_name: null,
  }
}

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
        consecutive_missed_backups: 0,
        missed_backup_threshold: 3,
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
//
// The demo also seeds its own never-resolving `started` backup report for
// web-server-01 / server-daily, attributed to the "Colliding daily window"
// schedule rather than schedule 1 (see seed-demo.sh) - so the real API
// response this mock builds on can already contain an entry for the same
// hostname/repo pair, just under a different `schedule_id`. Both
// DashboardView's `mergeActiveBackups` (keyed by `hostname::repo_name`) and
// HostsView's active-backup map (keyed by `hostname`+`repo_name`) treat two
// entries for that pair as two separate active backups, so filtering on
// `schedule_id !== 1` alone would miss the real entry and leave both it and
// this synthetic one in the response. Filter by hostname/repo instead, so
// only one entry for this pair ever survives, regardless of which schedule
// the real seed data currently attributes it to.
export async function mockRunningBackupOperation(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/stats/dashboard-overview',
    async (route) => {
      const response = await route.fetch()
      const overview = (await response.json()) as {
        running_operations: Array<{ hostname: unknown; repo_id: unknown }>
      }
      overview.running_operations = [
        ...overview.running_operations.filter(
          (op) => !(op.hostname === 'web-server-01' && op.repo_id === 1),
        ),
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

/**
 * Stubs the scope-option lookups the Notifications page fires on load (repos, agents,
 * schedules) as empty, for specs that only care about channels/history/content and don't
 * want those lists populated. Shared so notification-history.spec.ts and
 * notification-content.spec.ts don't each keep their own copy.
 */
export async function mockEmptyScopeOptionRoutes(page: Page): Promise<void> {
  await page.route('**/api/repos', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/agents', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/schedules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
}

// Archive host groups start collapsed once a repository spans more hosts than
// the grouping threshold, so .archive-row elements are hidden until their
// group is expanded. Wait for the list to settle into some terminal state
// first, since callers vary in how much they've already waited for the
// archives fetch to resolve.
export async function expandAllArchiveGroups(page: Page): Promise<void> {
  // The list renders skeleton rows while it loads, so waiting for "any of
  // archive-group / archive-row-detailed / empty-state" can resolve on a
  // transient placeholder and leave the groups collapsed for the rest of the
  // test. Wait the placeholders out explicitly first.
  await page
    .locator('.archive-loading')
    .waitFor({ state: 'hidden', timeout: 20_000 })
    .catch(() => {})

  await page
    .locator('.archive-group, .archive-row-detailed, .empty-state, .error-banner')
    .first()
    .waitFor({ state: 'visible', timeout: 20_000 })
    .catch(() => {})
  // A repository with more hosts than the grouping threshold starts collapsed;
  // click the toggles of the ones that are. The toggle is a control of its own
  // beside the host link, and its hit area covers the whole header.
  const collapsedToggles = page.locator('.group-header.collapsed .group-toggle')
  while ((await collapsedToggles.count()) > 0) {
    await collapsedToggles.first().click()
  }
}

/** What a spec built on [`mockNotificationsApi`] actually cares about. */
export interface NotificationApiMocks {
  channels: object[]
  deliveries: object[]
  rules?: object[]
}

// Routes every endpoint the Notifications view loads on open, so a spec only
// has to describe the channel and delivery it is actually about. Wraps
// [`mockEmptyScopeOptionRoutes`] rather than repeating it: every spec that
// mocks the notifications API wants those empty too.
export async function mockNotificationsApi(page: Page, mocks: NotificationApiMocks): Promise<void> {
  const json = (body: unknown): { status: number; contentType: string; body: string } => ({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify(body),
  })
  await page.route('**/api/notifications/channels', (route) => route.fulfill(json(mocks.channels)))
  await page.route('**/api/notifications/rules', (route) => route.fulfill(json(mocks.rules ?? [])))
  await page.route('**/api/notifications/deliveries*', (route) =>
    route.fulfill(json(mocks.deliveries)),
  )
  await page.route('**/api/notifications/push/vapid-key', (route) =>
    route.fulfill(json({ public_key: '', configured: false })),
  )
  await mockEmptyScopeOptionRoutes(page)
}

/**
 * Opens the Notifications view's History tab and returns its delivery rows,
 * already waited on so a caller can assert against the first one directly.
 */
export async function openNotificationHistory(page: Page): Promise<Locator> {
  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')
  await page.getByRole('tab', { name: 'History' }).click()
  const rows = page.locator('.delivery-row')
  await expect(rows.first()).toBeVisible({ timeout: 10_000 })
  return rows
}

// Intercepts a PUT to /api/schedules/:id, capturing the request body and
// handing both it and the real response body to `buildResponseBody` to shape
// what's echoed back - a real save round-trips through the schedule's other
// fields untouched, so a caller that only cares about one field merges its
// write into the original response rather than replacing it outright.
//
// Returns a function that resolves to the request body only once the route
// handler has actually called `route.fulfill()` - not just once the request
// body has been captured. Resolving early (while `route.fetch()` is still
// forwarding to the real backend) let the test finish and its page get torn
// down while that fetch was still in flight, which Playwright then aborts
// with "Target page, context or browser has been closed" from inside the
// route callback.
export async function interceptScheduleSave(
  page: Page,
  scheduleId: number,
  buildResponseBody: (
    requestBody: Record<string, unknown>,
    originalResponseBody: Record<string, unknown>,
  ) => Record<string, unknown>,
): Promise<() => Promise<Record<string, unknown>>> {
  let resolveSaved: (body: Record<string, unknown>) => void
  const saved = new Promise<Record<string, unknown>>((resolve) => {
    resolveSaved = resolve
  })
  await page.route(
    (url) => url.pathname === `/api/schedules/${scheduleId}`,
    async (route) => {
      if (route.request().method() === 'PUT') {
        const requestBody = (await route.request().postDataJSON()) as Record<string, unknown>
        const response = await route.fetch()
        const body = (await response.json()) as Record<string, unknown>
        await route.fulfill({
          status: response.status(),
          contentType: 'application/json',
          body: JSON.stringify(buildResponseBody(requestBody, body)),
        })
        resolveSaved(requestBody)
        return
      }
      return route.continue()
    },
  )
  return () => saved
}

/**
 * The id of a seeded schedule, looked up by name. The seed creates schedules in
 * an order other specs depend on, so a spec that needs one of the later ones
 * asks for it rather than assuming an id.
 */
export async function scheduleIdByName(page: Page, name: string): Promise<number> {
  const id = await page.evaluate(async (wanted) => {
    const response = await fetch('/api/schedules', { credentials: 'include' })
    const rows = (await response.json()) as { id: number; name: string }[]
    return rows.find((r) => r.name === wanted)?.id ?? 0
  }, name)
  expect(id, `seeded schedule "${name}" must exist`).toBeGreaterThan(0)
  return id
}
