// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { createHmac } from 'node:crypto'
import { type Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

function generateTotpCode(secretHex: string, stepOffset = 0): string {
  const secret = Buffer.from(secretHex, 'hex')
  const counter = Math.floor(Date.now() / 1000 / 30) + stepOffset
  const counterBuf = Buffer.alloc(8)
  counterBuf.writeBigUInt64BE(BigInt(counter), 0)
  const hmac = createHmac('sha1', secret).update(counterBuf).digest()
  const offset = hmac[hmac.length - 1] & 0x0f
  const code =
    (((hmac[offset] & 0x7f) << 24) |
      ((hmac[offset + 1] & 0xff) << 16) |
      ((hmac[offset + 2] & 0xff) << 8) |
      (hmac[offset + 3] & 0xff)) %
    1_000_000
  return code.toString().padStart(6, '0')
}

async function loginAsTotpUser(page: Page): Promise<void> {
  await page.goto('/login')
  await page.locator('input[type="text"], input[name="username"]').fill('totpuser')
  await page.locator('input[type="password"]').fill('totpuser')
  await Promise.all([
    page.waitForResponse(
      (resp) => resp.url().includes('/api/auth/login') && resp.status() === 200,
      { timeout: 60_000 },
    ),
    page.locator('button[type="submit"]').click(),
  ])
  await page.waitForURL((url) => !new URL(url).pathname.startsWith('/login'), {
    timeout: 60_000,
    waitUntil: 'commit',
  })
}

async function enrollTotp(page: Page): Promise<{ secret: string; recovery_codes: string[] }> {
  await page.goto('/profile')
  await page.waitForLoadState('networkidle')

  await page.getByRole('tab', { name: 'Two-factor auth' }).click()
  await page.getByRole('button', { name: 'Set up two-factor auth' }).click()

  const setupResponse = await page.waitForResponse(
    (resp) => resp.url().includes('/api/auth/totp/setup') && resp.status() === 200,
  )
  const setupBody = (await setupResponse.json()) as {
    secret: string
    qr_uri: string
    recovery_codes: string[]
  }
  expect(setupBody.secret).toHaveLength(40)
  expect(setupBody.recovery_codes.length).toBeGreaterThan(0)

  const code = generateTotpCode(setupBody.secret)
  await page.locator('input[placeholder="000000"]').fill(code)
  await page.getByRole('button', { name: 'Verify & Enable' }).click()

  // After enabling, the recovery codes are shown once for the user to save.
  await expect(page.locator('text=Recovery Codes').first()).toBeVisible({
    timeout: 10_000,
  })
  await page.getByRole('button', { name: 'I have saved these codes' }).click()

  await expect(page.locator('text=Two-factor authentication is enabled').first()).toBeVisible({
    timeout: 10_000,
  })

  return setupBody
}

async function logoutAndStartTotpLogin(page: Page): Promise<void> {
  await page.request.post('/api/auth/logout')

  await page.goto('/login')
  await page.locator('input[type="text"], input[name="username"]').fill('totpuser')
  await page.locator('input[type="password"]').fill('totpuser')
  await page.locator('button[type="submit"]').click()

  await expect(
    page.locator('text=Two-factor authentication is required for this account.').first(),
  ).toBeVisible({ timeout: 10_000 })
}

async function disableTotpIfEnabled(page: Page): Promise<void> {
  try {
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')
    const tab = page.getByRole('tab', { name: 'Two-factor auth' })
    if (!(await tab.isVisible().catch(() => false))) {
      return
    }
    await tab.click()
    const disableButton = page.getByRole('button', { name: 'Disable two-factor auth' })
    if (await disableButton.isVisible().catch(() => false)) {
      await page.locator('input[placeholder="Current password"]').fill('totpuser')
      await disableButton.click()
      await expect(
        page.locator('text=Two-factor authentication is not enabled').first(),
      ).toBeVisible({ timeout: 10_000 })
    }
  } catch {
    // Best-effort cleanup: ignore errors so a failed test does not mask the
    // original failure.
  }
}

test.describe('TOTP / 2FA', () => {
  test.afterEach(async ({ page }) => {
    await disableTotpIfEnabled(page)
  })

  test('Profile page shows TOTP enrollment option', async ({ page }) => {
    await loginAsTotpUser(page)
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('tab', { name: 'Two-factor auth' })).toBeVisible()
  })

  test('Sessions tab on profile shows active sessions', async ({ page }) => {
    await loginAsTotpUser(page)
    await page.goto('/profile')
    await page.waitForLoadState('networkidle')

    const sessionsTab = page.getByRole('tab', { name: 'Sessions' })
    await expect(sessionsTab).toBeVisible()
    await sessionsTab.click()
    await expect(page.locator('text=current').first()).toBeVisible()
  })

  test('TOTP enrollment and two-step login flow', async ({ page }) => {
    test.setTimeout(120_000)

    await loginAsTotpUser(page)
    const setupBody = await enrollTotp(page)

    await logoutAndStartTotpLogin(page)

    // Use the *next* time-step's code, not the current one: the server now
    // records the step consumed by the enrollment code itself (replay
    // protection covers it too), and this flow runs fast enough that
    // "current step" here could otherwise still be the same step just used
    // to enable TOTP a moment ago.
    const loginCode = generateTotpCode(setupBody.secret, 1)
    await page.locator('input[placeholder="000000"]').fill(loginCode)
    await page.locator('button[type="submit"]').click()

    await page.waitForURL((url) => url.pathname === '/', {
      timeout: 60_000,
      waitUntil: 'commit',
    })
  })

  test('can log in with a recovery code when the authenticator is unavailable', async ({
    page,
  }) => {
    test.setTimeout(120_000)

    await loginAsTotpUser(page)
    const setupBody = await enrollTotp(page)

    await logoutAndStartTotpLogin(page)

    await page.getByRole('button', { name: 'Use a recovery code instead' }).click()
    await expect(page.locator('input[placeholder="xxxxxxxx-xxxxxxxx"]')).toBeVisible()

    await page.locator('input[placeholder="xxxxxxxx-xxxxxxxx"]').fill(setupBody.recovery_codes[0]!)
    await page.locator('button[type="submit"]').click()

    await page.waitForURL((url) => url.pathname === '/', {
      timeout: 60_000,
      waitUntil: 'commit',
    })
  })
})

test('can revoke another active session from the Sessions tab', async ({ page, request }) => {
  await loginAsAdmin(page)

  // Create a second session for the same admin user via a cookie-isolated
  // API context so it shows up as a non-current, revocable session.
  await request.post('/api/auth/login', {
    data: { username: 'admin', password: 'admin', remember_me: false },
  })

  await page.goto('/profile')
  await page.waitForLoadState('networkidle')

  const sessionsTab = page.getByRole('tab', { name: 'Sessions' })
  await expect(sessionsTab).toBeVisible({ timeout: 10_000 })
  await sessionsTab.click()

  const revokeButtons = page.locator('button[title="Revoke session"]')
  await expect(revokeButtons.first()).toBeVisible({ timeout: 10_000 })
  const beforeCount = await revokeButtons.count()
  expect(beforeCount).toBeGreaterThan(0)

  await revokeButtons.first().click()
  await page.getByRole('button', { name: 'Revoke', exact: true }).click()

  await expect(async () => {
    expect(await revokeButtons.count()).toBe(beforeCount - 1)
  }).toPass({ timeout: 10_000 })
})

test.describe('Session idle timeout', () => {
  test('Session idle timeout can be configured on System Settings', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/system')
    await page.waitForLoadState('networkidle')

    const input = page.locator('#settings-idle-timeout')
    await expect(input).toBeVisible({ timeout: 10_000 })
    await expect(input).toHaveValue('480')

    await input.fill('60')
    await page.getByRole('button', { name: 'Save' }).click()
    await expect(page.locator('text=Settings saved').first()).toBeVisible({
      timeout: 10_000,
    })

    // Reset to the default so later tests keep the standard 8-hour window.
    await input.fill('480')
    await page.getByRole('button', { name: 'Save' }).click()
    await expect(page.locator('text=Settings saved').first()).toBeVisible({
      timeout: 10_000,
    })
  })
})
