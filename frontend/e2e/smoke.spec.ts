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

// waitForApi identifies the request each route's initial mount fires, so the
// test can wait for the actual response instead of guessing from DOM state.
// `waitUntil: 'commit'` resolves before Vue even mounts, so a page-goto
// immediately followed by a DOM assertion (e.g. "no loading spinner visible")
// can pass before the fetch has *started*, not after it finished - a
// false-negative race, not a real wait. Racing page.goto against
// page.waitForResponse instead ties completion to the network response
// itself, so the request (and the backend handler serving it) has
// deterministically finished by the time the test - and eventually the
// suite - ends. Otherwise whether the handler's lines run to completion
// before teardown is a scheduling race, not a deterministic outcome - see
// #366, and the get_database_storage regression on #365/#378 specifically.
const routes = [
  { path: '/agents', label: 'agents list', waitForApi: '/api/agents' },
  { path: '/repos', label: 'repos list', waitForApi: '/api/repos/stats' },
  { path: '/schedules', label: 'schedules list', waitForApi: '/api/schedules' },
  { path: '/activity', label: 'activity log', waitForApi: '/api/logs' },
  { path: '/tokens', label: 'tokens page', waitForApi: '/api/tokens' },
  { path: '/profile', label: 'profile page', waitForApi: '/api/tokens' },
]

for (const { path, label, waitForApi } of routes) {
  test(`${label} loads without error`, async ({ page }) => {
    await loginAsAdmin(page)
    await Promise.all([
      page.waitForResponse((res) => res.url().includes(waitForApi)),
      page.goto(path, { waitUntil: 'commit' }),
    ])
    await expect(page).not.toHaveURL(/\/error/, { timeout: 15_000 })
    await expect(page).toHaveURL(new RegExp(path))
  })
}

const adminRoutesWithLabels = [
  { path: '/system', label: 'system settings', waitForApi: '/api/system/database-storage' },
  { path: '/admin/roles', label: 'roles management', waitForApi: '/api/roles' },
  { path: '/admin/groups', label: 'groups management', waitForApi: '/api/groups' },
  { path: '/notifications', label: 'notifications config', waitForApi: '/api/notifications/channels' },
  { path: '/audit-log', label: 'audit log', waitForApi: '/api/audit-log' },
]

for (const { path, label, waitForApi } of adminRoutesWithLabels) {
  test(`admin: ${label} loads without error`, async ({ page }) => {
    await loginAsAdmin(page)
    await Promise.all([
      page.waitForResponse((res) => res.url().includes(waitForApi)),
      page.goto(path, { waitUntil: 'commit' }),
    ])
    await expect(page).not.toHaveURL(/\/error/, { timeout: 15_000 })
    await expect(page).toHaveURL(new RegExp(path))
  })
}

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
