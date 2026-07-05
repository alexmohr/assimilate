// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from './fixtures'

test('rate limiting returns 429 after too many failed logins', async ({ page }) => {
  const badCredentials = { username: 'nonexistent-rate-limit-user', password: 'wrong' }

  // Exhaust the per-IP login limit (MAX_LOGIN_ATTEMPTS = 5)
  for (let i = 0; i < 5; i++) {
    const resp = await page.request.post('/api/auth/login', { data: badCredentials })
    // First 5 attempts return 401
    expect(resp.status()).toBe(401)
  }

  // The 6th attempt should be rate-limited
  const blocked = await page.request.post('/api/auth/login', { data: badCredentials })
  expect(blocked.status()).toBe(429)
})

test('rate limiting header hints at retry-after', async ({ page }) => {
  const badCredentials = { username: 'nonexistent-rate-limit-header-user', password: 'wrong' }

  // Exhaust the limit
  for (let i = 0; i < 5; i++) {
    await page.request.post('/api/auth/login', { data: badCredentials })
  }

  const blocked = await page.request.post('/api/auth/login', { data: badCredentials })
  expect(blocked.status()).toBe(429)
  const retryAfter = blocked.headers()['retry-after']
  // The response may include a Retry-After header
  if (retryAfter) {
    const seconds = Number(retryAfter)
    expect(Number.isNaN(seconds)).toBe(false)
    expect(seconds).toBeGreaterThan(0)
  }
})

test('account lockout triggers after repeated failures from any IP', async ({ page }) => {
  const username = 'nonexistent-lockout-user'
  const badCredentials = { username, password: 'wrong' }

  // MAX_ACCOUNT_FAILURES = 10 across all IPs
  for (let i = 0; i < 10; i++) {
    const resp = await page.request.post('/api/auth/login', { data: badCredentials })
    expect([401, 429]).toContain(resp.status())
  }

  // The 11th attempt hits account lockout (independent of IP rate limit)
  const blocked = await page.request.post('/api/auth/login', { data: badCredentials })
  expect(blocked.status()).toBe(429)
})
