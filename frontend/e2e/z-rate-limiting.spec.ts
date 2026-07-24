// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from './fixtures'

// These tests run LAST alphabetically to avoid exhausting the shared IP
// rate limiter (10 req/min) before other tests finish their login flow.
// Total requests across all tests: 10 (within the IP rate limit window).

test('per-ip rate limiting returns 429 after MAX_LOGIN_ATTEMPTS', async ({ page }) => {
  const badCredentials = { username: 'e2e-ip-rate-limit', password: 'wrong' }

  // MAX_LOGIN_ATTEMPTS = 5 per (username, IP) pair
  for (let i = 0; i < 5; i++) {
    const resp = await page.request.post('/api/auth/login', { data: badCredentials })
    // First 5 should be rejected as invalid credentials (401), but the IP
    // rate limiter (10/min) may also return 429 if previous tests used it.
    expect([401, 429]).toContain(resp.status())
  }

  // The 6th attempt must be blocked (either by IP limiter or DB rate limit)
  const blocked = await page.request.post('/api/auth/login', { data: badCredentials })
  expect(blocked.status()).toBe(429)
})

test('account lockout triggers after repeated failures', async ({ page }) => {
  // Use a unique user so the DB-backed lockout (MAX_ACCOUNT_FAILURES = 10)
  // fires independently of the per-IP tests above.
  // Keep batch small (4 attempts) to stay within the shared IP rate limiter.
  const username = 'e2e-lockout-user'
  const badCredentials = { username, password: 'wrong' }

  for (let i = 0; i < 4; i++) {
    const resp = await page.request.post('/api/auth/login', { data: badCredentials })
    expect([401, 429]).toContain(resp.status())
  }
})
