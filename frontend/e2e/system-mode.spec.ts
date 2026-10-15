// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'
import type { SystemModeResponse } from '../src/types/generated/SystemModeResponse'

test('deployment mode is readable before login and defaults to server', async ({ page }) => {
  const resp = await page.request.get('/api/system/mode')
  expect(resp.status()).toBe(200)
  const body = (await resp.json()) as SystemModeResponse
  expect(body.mode).toBe('server')
})

// The demo server runs in server mode, so desktop mode is simulated in the
// browser. Only the UI changes between modes; the server's answer is all it reads.
async function pretendDesktopMode(page: Page): Promise<void> {
  const desktop: SystemModeResponse = { mode: 'desktop' }
  await page.route('**/api/system/mode', (route) => route.fulfill({ json: desktop }))
}

async function openSettingsNav(page: Page): Promise<void> {
  await page.locator('.nav').getByRole('button', { name: 'Settings' }).click()
}

test.describe('desktop mode', () => {
  test('hides user management, RBAC and tunnels from the sidebar', async ({ page }) => {
    await pretendDesktopMode(page)
    await loginAsAdmin(page)
    await openSettingsNav(page)

    const nav = page.locator('.nav')
    await expect(nav.getByRole('link', { name: 'Dashboard' })).toBeVisible()
    await expect(nav.getByRole('link', { name: 'System' })).toBeVisible()
    await expect(nav.getByRole('link', { name: 'Audit Log' })).toBeVisible()
    await expect(nav.getByRole('link', { name: 'Tunnels' })).toHaveCount(0)
    await expect(nav.getByRole('link', { name: 'Users' })).toHaveCount(0)
    await expect(nav.getByRole('link', { name: 'Groups' })).toHaveCount(0)
    await expect(nav.getByRole('link', { name: 'Roles' })).toHaveCount(0)
  })

  // The mode answer is held back so a sidebar that rendered before knowing it
  // would show the Tunnels link meanwhile. An observer installed before the app
  // loads records whether it ever appeared, so a brief flash fails the test.
  test('never renders hidden pages while the mode is loading', async ({ page }) => {
    const desktop: SystemModeResponse = { mode: 'desktop' }
    await page.route('**/api/system/mode', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1_500))
      await route.fulfill({ json: desktop })
    })
    await page.addInitScript(() => {
      const flags = window as unknown as { tunnelsLinkSeen: boolean }
      flags.tunnelsLinkSeen = false
      new MutationObserver(() => {
        if (document.querySelector('.nav a[href="/tunnels"]')) {
          flags.tunnelsLinkSeen = true
        }
      }).observe(document, { childList: true, subtree: true })
    })
    await loginAsAdmin(page)

    await page.goto('/')
    await expect(page.locator('.nav').getByRole('link', { name: 'Dashboard' })).toBeVisible()

    const seen = await page.evaluate(
      () => (window as unknown as { tunnelsLinkSeen: boolean }).tunnelsLinkSeen,
    )
    expect(seen).toBe(false)
  })

  test('redirects a server-only page to the dashboard', async ({ page }) => {
    await pretendDesktopMode(page)
    await loginAsAdmin(page)

    await page.goto('/users')

    await expect(page).toHaveURL(/\/$/)
  })

  // The desktop app signs in with only a password, so enrolling TOTP there
  // would lock it out; the demo admin has no TOTP, so the tab is gone.
  test('hides the two-factor tab on the profile page', async ({ page }) => {
    await pretendDesktopMode(page)
    await loginAsAdmin(page)

    await page.goto('/profile')

    await expect(page.getByRole('tab', { name: 'Change password' })).toBeVisible()
    await expect(page.getByRole('tab', { name: 'Sessions' })).toBeVisible()
    await expect(page.getByRole('tab', { name: 'Two-factor auth' })).toHaveCount(0)
  })

  test('server mode still offers every page', async ({ page }) => {
    await loginAsAdmin(page)
    await openSettingsNav(page)

    const nav = page.locator('.nav')
    await expect(nav.getByRole('link', { name: 'Tunnels' })).toBeVisible()
    await expect(nav.getByRole('link', { name: 'Users' })).toBeVisible()
  })

  test('server mode still offers two-factor auth on the profile page', async ({ page }) => {
    await loginAsAdmin(page)

    await page.goto('/profile')

    await expect(page.getByRole('tab', { name: 'Two-factor auth' })).toBeVisible()
  })
})
