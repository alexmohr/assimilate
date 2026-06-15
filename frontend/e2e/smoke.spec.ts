// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test, type Page } from '@playwright/test'

async function loginAsAdmin(page: Page): Promise<void> {
  await page.goto('/login')
  await page.locator('input[type="text"], input[name="username"]').fill('admin')
  await page.locator('input[type="password"]').fill('admin')
  await page.locator('button[type="submit"]').click()
  await page.waitForURL((url) => !new URL(url).pathname.startsWith('/login'), { timeout: 30_000 })
}

// ── Unauthenticated ──────────────────────────────────────────────────────────

test('login page loads', async ({ page }) => {
  await page.goto('/login')
  await expect(page.locator('input[type="password"]')).toBeVisible()
})

test('unauthenticated users are redirected to login', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveURL(/\/login/)
})

// ── Authentication ───────────────────────────────────────────────────────────

test('admin can log in and reach the dashboard', async ({ page }) => {
  await loginAsAdmin(page)
  await expect(page).not.toHaveURL(/\/error/)
  await expect(page).not.toHaveURL(/\/login/)
})

test('logout redirects to login', async ({ page }) => {
  await loginAsAdmin(page)
  // Trigger logout via the API directly so we don't depend on nav UI details
  await page.request.post('/api/auth/logout')
  await page.goto('/')
  await expect(page).toHaveURL(/\/login/)
})

// ── Dashboard renders without errors ────────────────────────────────────────

test('dashboard does not redirect to /error', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  // Give async data fetches time to settle
  await page.waitForTimeout(2_000)
  await expect(page).not.toHaveURL(/\/error/)
})

test('dashboard renders content panels', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  // At least one panel heading must be present once the page settles
  await expect(page.locator('h2').first()).toBeVisible({ timeout: 10_000 })
})

// ── Navigation: every main route loads without throwing ─────────────────────

const routes = [
  { path: '/agents', label: 'agents list' },
  { path: '/repos', label: 'repos list' },
  { path: '/schedules', label: 'schedules list' },
  { path: '/activity', label: 'activity log' },
  { path: '/tokens', label: 'tokens page' },
  { path: '/profile', label: 'profile page' },
]

for (const { path, label } of routes) {
  test(`${label} loads without error`, async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(path)
    await page.waitForTimeout(2_000)
    await expect(page).not.toHaveURL(/\/error/)
    await expect(page).toHaveURL(new RegExp(path))
  })
}

// ── Admin-only routes ────────────────────────────────────────────────────────

const adminRoutes = [
  { path: '/system', label: 'system settings' },
  { path: '/admin/roles', label: 'roles management' },
  { path: '/admin/groups', label: 'groups management' },
  { path: '/notifications', label: 'notifications config' },
  { path: '/audit-log', label: 'audit log' },
]

for (const { path, label } of adminRoutes) {
  test(`admin: ${label} loads without error`, async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto(path)
    await page.waitForTimeout(2_000)
    await expect(page).not.toHaveURL(/\/error/)
    await expect(page).toHaveURL(new RegExp(path))
  })
}

// ── Demo data is visible ─────────────────────────────────────────────────────

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
