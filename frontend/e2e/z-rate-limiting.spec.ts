// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from './fixtures'

// This test runs LAST alphabetically to avoid exhausting the shared IP
// rate limiter before other tests finish their login flow.
//
// The account-wide lockout (MAX_ACCOUNT_FAILURES = 10 failures, tracked per
// username across all IPs, independent of the per-(username, IP) limiter
// below) is *not* covered here: every request in a Playwright run shares one
// real source IP, and MAX_LOGIN_ATTEMPTS (5 failures per username+IP within
// LOGIN_WINDOW_MINUTES) always blocks a 6th same-IP attempt with 429 before
// the account-wide counter can ever reach 10. Driving and verifying the full
// lockout (including that it also rejects a *correct* password, the only
// observable signal -- locked and wrong-password both return an identical
// 401 by design) needs varying the caller's IP per batch, which requires
// direct control over the request's `ConnectInfo`. See
// `test_account_lockout_rejects_correct_password_while_locked` in
// `crates/server/tests/integration.rs` for that coverage.

test('per-ip rate limiting returns 429 after MAX_LOGIN_ATTEMPTS', async ({ page }) => {
  const badCredentials = { username: 'e2e-ip-rate-limit', password: 'wrong' }

  // MAX_LOGIN_ATTEMPTS = 5 per (username, IP) pair
  for (let i = 0; i < 5; i++) {
    const resp = await page.request.post('/api/auth/login', { data: badCredentials })
    // First 5 should be rejected as invalid credentials (401), but the IP
    // rate limiter may also return 429 if previous tests used it.
    expect([401, 429]).toContain(resp.status())
  }

  // The 6th attempt must be blocked (either by IP limiter or DB rate limit)
  const blocked = await page.request.post('/api/auth/login', { data: badCredentials })
  expect(blocked.status()).toBe(429)
})
