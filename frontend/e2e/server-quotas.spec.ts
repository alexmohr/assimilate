// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

const SHARED_HOST_QUOTA = {
  ssh_host: 'storage.example.com',
  repo_count: 2,
  total_deduplicated_size: 5_368_709_120,
  configured: true,
  warn_bytes: 8_589_934_592,
  critical_bytes: 10_737_418_240,
  warn_action: 'notify_only',
  critical_action: 'block_backups',
  enabled: true,
  updated_at: '2026-07-01T00:00:00Z',
}

async function interceptServerQuotasApi(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/server-quotas',
    async (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([SHARED_HOST_QUOTA]),
      }),
  )
  await page.route(
    (url) => url.pathname === '/api/server-quotas/storage.example.com',
    async (route) => {
      if (route.request().method() === 'PUT') {
        const body = (await route.request().postDataJSON()) as Record<string, unknown>
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...SHARED_HOST_QUOTA, ...body }),
        })
      }
      return route.continue()
    },
  )
}

test('server quotas page shows shared host usage and configured actions', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptServerQuotasApi(page)

  await page.goto('/server-quotas')
  await expect(page.getByRole('heading', { name: 'Server Quotas' })).toBeVisible()
  await expect(page.getByText('storage.example.com')).toBeVisible()
  await expect(page.getByText('Block backups')).toBeVisible()
})

test('admin can edit a server quota action and save', async ({ page }) => {
  await loginAsAdmin(page)
  await interceptServerQuotasApi(page)

  await page.goto('/server-quotas')
  await expect(page.getByText('storage.example.com')).toBeVisible()

  await page.getByRole('button', { name: 'Edit' }).click()
  await expect(page.getByRole('heading', { name: 'Quota for storage.example.com' })).toBeVisible()

  let savedBody: Record<string, unknown> | null = null
  await page.route(
    (url) => url.pathname === '/api/server-quotas/storage.example.com',
    async (route) => {
      if (route.request().method() === 'PUT') {
        savedBody = (await route.request().postDataJSON()) as Record<string, unknown>
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ ...SHARED_HOST_QUOTA, ...savedBody }),
        })
      }
      return route.continue()
    },
  )

  await page.getByLabel('Warning action').selectOption('disable_schedule')
  await page.getByRole('button', { name: 'Save' }).click()

  await expect(async () => {
    expect(savedBody).not.toBeNull()
    expect((savedBody as Record<string, unknown>).warn_action).toBe('disable_schedule')
  }).toPass({ timeout: 5_000 })
})

test('settings submenu contains server quotas link for admin', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/')
  await page.getByRole('button', { name: 'Settings' }).click()
  await expect(page.getByRole('link', { name: 'Server Quotas' })).toBeVisible()
})
