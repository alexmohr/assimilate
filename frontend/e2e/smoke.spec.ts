// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('login page loads', async ({ page }) => {
  await page.goto('/login')
  await expect(page.locator('input[type="password"]')).toBeVisible()
})

test('unauthenticated users are redirected to login', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveURL(/\/login/)
})

test('admin can log in and reach the dashboard', async ({ page }) => {
  await loginAsAdmin(page)
  await expect(page).not.toHaveURL(/\/error/)
  await expect(page).not.toHaveURL(/\/login/)
})

test('logout redirects to login', async ({ page }) => {
  await loginAsAdmin(page)
  // Trigger logout via the API directly so we don't depend on nav UI details
  await page.request.post('/api/auth/logout')
  // waitUntil: 'commit' resolves on response headers; without it Playwright
  // throws ERR_ABORTED when the SPA route guard fires a client-side redirect
  // before the page finishes loading.
  await page.goto('/', { waitUntil: 'commit' })
  await expect(page).toHaveURL(/\/login/, { timeout: 10_000 })
})

test('dashboard does not redirect to /error', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  // Wait for the page to finish loading async data
  await expect(page.locator('h2').first())
    .toBeVisible({ timeout: 10_000 })
    .catch(() => {})
  await expect(page).not.toHaveURL(/\/error/)
})

test('dashboard renders content panels', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  // At least one panel heading must be present once the page settles
  await expect(page.locator('h2').first()).toBeVisible({ timeout: 10_000 })
})

// waitForApi identifies the request(s) each route's initial mount fires, so
// the test can wait for the actual response(s) instead of guessing from DOM
// state. `waitUntil: 'commit'` resolves before Vue even mounts, so a
// page-goto immediately followed by a DOM assertion (e.g. "no loading
// spinner visible") can pass before the fetch has *started*, not after it
// finished - a false-negative race, not a real wait. Racing page.goto
// against page.waitForResponse instead ties completion to the network
// response itself, so the request (and the backend handler serving it) has
// deterministically finished by the time the test - and eventually the
// suite - ends. Otherwise whether the handler's lines run to completion
// before teardown is a scheduling race, not a deterministic outcome - see
// #366, and the get_database_storage regression on #365/#378 specifically.
//
// A single string is only correct when the view fires exactly one request
// (or a sequential chain ending in that request) on mount - waitForResponse
// resolves on the *first* match, so a view that fires several concurrent,
// independent requests needs every one of them listed, or the test still
// finishes (and can race server-side completion) before the others do.
const routes: Array<{ path: string; label: string; waitForApi: string | string[] }> = [
  { path: '/agents', label: 'agents list', waitForApi: '/api/agents' },
  { path: '/repos', label: 'repos list', waitForApi: '/api/repos/stats' },
  { path: '/schedules', label: 'schedules list', waitForApi: '/api/schedules' },
  { path: '/activity', label: 'activity log', waitForApi: '/api/stats/activity' },
  { path: '/tokens', label: 'tokens page', waitForApi: '/api/tokens' },
  { path: '/profile', label: 'profile page', waitForApi: '/api/tokens' },
  { path: '/system', label: 'admin: system settings', waitForApi: '/api/system/database-storage' },
  { path: '/admin/roles', label: 'admin: roles management', waitForApi: '/api/roles' },
  // NotificationsView's onMounted fires four independent, uncoordinated
  // loaders with no shared loading flag: channels+rules, deliveries,
  // push/vapid-key, and repos+agents+schedules (for scope pickers) - list
  // every endpoint they hit so all of them are awaited, not just the first.
  // loadChannels itself is sequential (channels, then rules only after
  // channels resolves), so both need listing or waitForResponse can
  // resolve on the channels response alone.
  {
    path: '/notifications',
    label: 'admin: notifications config',
    waitForApi: [
      '/api/notifications/channels',
      '/api/notifications/rules',
      '/api/notifications/deliveries',
      '/api/notifications/push/vapid-key',
      '/api/repos',
      '/api/agents',
      '/api/schedules',
    ],
  },
  { path: '/audit-log', label: 'admin: audit log', waitForApi: '/api/audit-log' },
]

for (const { path, label, waitForApi } of routes) {
  test(`${label} loads without error`, async ({ page }) => {
    await loginAsAdmin(page)
    const apis = Array.isArray(waitForApi) ? waitForApi : [waitForApi]
    await Promise.all([
      ...apis.map((api) =>
        page.waitForResponse((res) => res.url().includes(api), { timeout: 15_000 }),
      ),
      page.goto(path, { waitUntil: 'commit' }),
    ])
    await expect(page).not.toHaveURL(/\/error/, { timeout: 15_000 })
    await expect(page).toHaveURL(new RegExp(path))
  })
}

test('admin: groups management loads without error', async ({ page }) => {
  await loginAsAdmin(page)
  // GroupsView.onMounted does Promise.all([fetchGroups(), fetchUsers()]).
  // fetchUsers() (GET /users) is a fully independent request not gated by
  // fetchGroups's `loading` ref, so it needs its own network-level wait
  // alongside the spinner wait below - otherwise the test could finish
  // while /api/users is still in flight on the server.
  await Promise.all([
    page.waitForResponse((res) => res.url().includes('/api/users'), { timeout: 15_000 }),
    page.goto('/admin/groups', { waitUntil: 'commit' }),
  ])
  await expect(page).not.toHaveURL(/\/error/, { timeout: 15_000 })
  await expect(page).toHaveURL(/\/admin\/groups/)
  // fetchGroups fetches the group list, then a per-group member count for
  // however many groups the demo seeds - a dynamic fan-out that a fixed
  // waitForApi list can't enumerate. Its shared `loading` ref (driving this
  // BaseSpinner) only clears once that whole chain resolves, so wait for
  // the spinner to appear (it may not have rendered yet at this point -
  // `waitUntil: 'commit'` resolves before Vue mounts) and then clear,
  // rather than asserting "0 spinners" which could be trivially true
  // before the fetch even starts.
  await page
    .locator('[role="status"]')
    .first()
    .waitFor({ state: 'attached', timeout: 5_000 })
    .catch(() => {})
  await expect(page.locator('[role="status"]')).toHaveCount(0, { timeout: 15_000 })
})

test('agents page lists seeded agents', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  // Demo seeds web-server-01, db-server-01, media-store-01
  await expect(page.getByText('web-server-01')).toBeVisible({ timeout: 10_000 })
})

test('repos page lists seeded repositories', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await expect(page.getByText('server-daily')).toBeVisible({ timeout: 10_000 })
})

test('schedules page lists seeded schedules', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/schedules')
  // Demo seeds at least one schedule (cards rendered with .schedule-card class)
  await expect(page.locator('.schedule-card').first()).toBeVisible({ timeout: 10_000 })
})
