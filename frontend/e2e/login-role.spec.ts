// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test('login response includes role field in user object', async ({ page }) => {
  // Capture the login API response without intercepting the request
  const responsePromise = page.waitForResponse(
    (resp) => resp.url().includes('/api/auth/login') && resp.status() === 200,
  )

  await loginAsAdmin(page)

  const response = await responsePromise
  const body = (await response.json()) as Record<string, unknown>
  expect(body).toHaveProperty('user')
  const user = body.user as Record<string, unknown> | undefined
  expect(user).toHaveProperty('role')
  expect(user!.role).toBe('admin')
})

test('isAdmin is true immediately after login (admin nav items visible)', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')

  // Give the SPA time to settle after route transition
  await page.waitForTimeout(1_500)

  // Admin-only nav items (System, Users, Roles, etc.) are nested inside a
  // collapsible "Settings" group that defaults to collapsed.  Click the toggle
  // to expand it, then verify a System link is visible.
  const settingsToggle = page.locator('.nav-group-toggle')
  await expect(settingsToggle.first()).toBeVisible({ timeout: 10_000 })
  await settingsToggle.first().click()
  await page.waitForTimeout(500)

  await expect(page.locator('a[href="/system"]')).toBeVisible({ timeout: 5_000 })
})

test('login API response contract matches AuthUser interface', async ({ page }) => {
  // Verify the login endpoint's JSON shape aligns with the frontend AuthUser
  // interface by calling the API directly.  This covers the same code path
  // that non-admin users traverse, asserting that all required fields
  // (id, username, role, must_change_password, created_at, last_login_at)
  // are present and correctly typed.
  const loginResp = await page.request.post('/api/auth/login', {
    data: { username: 'admin', password: 'admin', remember_me: false },
  })
  expect(loginResp.status()).toBe(200)

  const body = (await loginResp.json()) as Record<string, unknown>
  expect(body).toHaveProperty('user')
  expect(body).toHaveProperty('session_expires_at')
  expect(body).toHaveProperty('remember_me')

  const user = body.user as Record<string, unknown> | undefined
  // All AuthUser fields must be present
  expect(user).toHaveProperty('id')
  expect(typeof user!.id).toBe('number')
  expect(user).toHaveProperty('username')
  expect(typeof user!.username).toBe('string')
  expect(user).toHaveProperty('role')
  expect(typeof user!.role).toBe('string')
  expect(user).toHaveProperty('must_change_password')
  expect(typeof user!.must_change_password).toBe('boolean')
  expect(user).toHaveProperty('created_at')
  expect(typeof user!.created_at).toBe('string')
  expect(user).toHaveProperty('last_login_at')
  // last_login_at is nullable - accept either string or null
  expect(user!.last_login_at === null || typeof user!.last_login_at === 'string').toBe(true)
})
