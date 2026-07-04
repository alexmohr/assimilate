// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from './fixtures'

// Use a non-existent username so these tests don't lock the admin account
// and break subsequent e2e tests.
const NONEXISTENT_USER = 'nonexistent-rate-limit-test-user'

test('too many failed login attempts returns 429', async ({ page }) => {
  let got429 = false

  for (let i = 0; i < 15; i++) {
    const resp = await page.request.post('/api/auth/login', {
      data: { username: NONEXISTENT_USER, password: 'wrong-password' },
    })
    if (resp.status() === 429) {
      got429 = true
      break
    }
  }

  expect(got429).toBe(true)
})

test('rate limited requests return same error format', async ({ page }) => {
  // Exhaust the IP rate limiter with a non-existent user
  let resp
  for (let i = 0; i < 10; i++) {
    resp = await page.request.post('/api/auth/login', {
      data: { username: NONEXISTENT_USER, password: 'wrong-password' },
      headers: { 'Content-Type': 'application/json' },
    })
  }

  // Verify the 429 response has the expected structure
  expect(resp!.status()).toBe(429)
  const body = await resp!.json()
  expect(body).toHaveProperty('error')
})
