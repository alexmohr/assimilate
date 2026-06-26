// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

async function interceptSystemApis(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/system/ssh-public-key',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA test-key' }),
      }),
  )
  await page.route(
    (url) => url.pathname === '/api/system/settings',
    async (route) => {
      if (route.request().method() === 'GET') {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            timezone: 'UTC',
            retention_days: 7,
            borg_query_timeout_secs: 300,
          }),
        })
      }
      return route.continue()
    },
  )
  await page.route(
    (url) => url.pathname === '/api/system/version',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          server_version: '0.1.0',
          server_git_sha: '',
          build_timestamp: 'unknown',
          server_commit_count: null,
          agent_version: null,
        }),
      }),
  )
  await page.route(
    (url) => url.pathname === '/api/system/database-storage',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ database_bytes: 0, other_bytes: 0, relations: [] }),
      }),
  )
}

test('system settings page renders borg timeout input', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptSystemApis(page)
  await page.goto('/system')
  await expect(page.locator('#settings-borg-timeout')).toBeVisible({ timeout: 10_000 })
  await expect(page.locator('#settings-borg-timeout')).toHaveValue('300')
})

test('admin can update borg timeout and save settings', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptSystemApis(page)

  let savedBody: Record<string, unknown> | null = null
  await page.route(
    (url) => url.pathname === '/api/system/settings',
    async (route) => {
      if (route.request().method() === 'PUT') {
        savedBody = (await route.request().postDataJSON()) as Record<string, unknown>
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            timezone: 'UTC',
            retention_days: 7,
            borg_query_timeout_secs: 600,
          }),
        })
      }
      return route.continue()
    },
  )

  await page.goto('/system')
  const input = page.locator('#settings-borg-timeout')
  await expect(input).toBeVisible({ timeout: 10_000 })

  await input.fill('600')
  await page.getByRole('button', { name: 'Save' }).click()

  await expect(async () => {
    expect(savedBody).not.toBeNull()
    expect((savedBody as Record<string, unknown>).borg_query_timeout_secs).toBe(600)
  }).toPass({ timeout: 5_000 })
})
