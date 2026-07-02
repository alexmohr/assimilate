// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from '@playwright/test'

test('service worker registers and supports push', async ({ page }) => {
  await page.goto('/login')

  const hasPushManager = await page.evaluate(async () => {
    if (!('serviceWorker' in navigator)) return false
    const registration = await navigator.serviceWorker.ready
    return 'pushManager' in registration
  })

  expect(hasPushManager).toBe(true)
})

test('service worker activates with push scope', async ({ page }) => {
  await page.goto('/login')

  const swState = await page.evaluate(async () => {
    if (!('serviceWorker' in navigator)) return null
    const registration = await navigator.serviceWorker.ready
    return registration.active?.state ?? null
  })

  expect(swState).toBe('activated')
})
