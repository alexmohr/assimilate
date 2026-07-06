// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { adminRoutes, expect, loginAsAdmin, test, verifyRedirectFromAdminRoutes } from './fixtures'

test('login response includes role field in user object', async ({ page }) => {
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

  const settingsToggle = page.locator('.nav-group-toggle')
  await expect(settingsToggle.first()).toBeVisible({ timeout: 10_000 })
  await settingsToggle.first().click()

  await expect(page.locator('a[href="/system"]')).toBeVisible({ timeout: 5_000 })
})

test('login API response contract matches AuthUser interface', async ({ page }) => {
  const loginResp = await page.request.post('/api/auth/login', {
    data: { username: 'admin', password: 'admin', remember_me: false },
  })
  expect(loginResp.status()).toBe(200)

  const body = (await loginResp.json()) as Record<string, unknown>
  expect(body).toHaveProperty('user')
  expect(body).toHaveProperty('session_expires_at')
  expect(body).toHaveProperty('remember_me')

  const user = body.user as Record<string, unknown> | undefined
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
  expect(user!.last_login_at === null || typeof user!.last_login_at === 'string').toBe(true)
})

test('viewer user login returns viewer role and cannot access admin pages', async ({ page }) => {
  // Log in as admin to set up a viewer test user
  await loginAsAdmin(page)

  const USERNAME = 'e2e-viewer-test'
  const PASSWORD = 'viewer-test-pw'

  // Create a test user via the admin API
  const createResp = await page.request.post('/api/users', {
    data: { username: USERNAME, password: PASSWORD },
  })
  expect(createResp.status()).toBe(201)
  const createdUser = (await createResp.json()) as { id: number; username: string }
  const userId = createdUser.id

  // Look up the viewer role ID
  const rolesResp = await page.request.get('/api/roles')
  expect(rolesResp.status()).toBe(200)
  const roles = (await rolesResp.json()) as Array<{ id: number; name: string }>
  const viewerRole = roles.find((r: { name: string }) => r.name === 'viewer')
  expect(viewerRole).toBeDefined()

  // Assign the viewer role to the test user
  const assignResp = await page.request.put(`/api/users/${userId}/roles`, {
    data: { role_ids: [viewerRole!.id] },
  })
  expect(assignResp.status()).toBe(204)

  // Log out admin
  await page.request.post('/api/auth/logout')

  // Log in as the viewer user and verify the role in the response
  const loginResp = await page.request.post('/api/auth/login', {
    data: { username: USERNAME, password: PASSWORD, remember_me: false },
  })
  expect(loginResp.status()).toBe(200)

  const body = (await loginResp.json()) as Record<string, unknown>
  expect(body).toHaveProperty('user')
  const user = body.user as Record<string, unknown> | undefined
  expect(user).toHaveProperty('role')
  expect(user!.role).toBe('viewer')

  // Navigate to non-admin page to verify authentication works
  await page.goto('/activity', { waitUntil: 'commit' })
  await expect(page).toHaveURL(/\/activity/, { timeout: 10_000 })

  // Verify viewer cannot access admin pages (redirected to dashboard)
  await verifyRedirectFromAdminRoutes(page, adminRoutes)

  // Clean up: re-login as admin and delete the test user
  await page.request.post('/api/auth/logout')
  await loginAsAdmin(page)
  const deleteResp = await page.request.delete(`/api/users/${userId}`)
  expect(deleteResp.status()).toBe(204)
})
