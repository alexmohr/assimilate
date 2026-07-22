// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

const LONG_ERROR =
  'web push transport error: failed to connect to the server -> [7] Could not connect to server (raw TCP probe: [2001:4860:4802:36::39]:443 failed: Network is unreachable (os error 101); 216.239.36.55:443 connected)'

function makeChannel(): object {
  return {
    id: 1,
    name: 'Ops Web Push',
    channel_type: 'web_push',
    config: { user_id: 1 },
    enabled: true,
    scope: {},
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  }
}

function makeDelivery(): object {
  return {
    id: 1,
    channel_id: 1,
    event_type: 'backup_failed',
    payload: {
      event_type: 'backup_failed',
      hostname: 'web-server-01',
      repo_name: 'daily-backup',
      status: 'failed',
      error_message: 'repository is locked',
      timestamp: '2026-01-15T03:00:12Z',
    },
    status: 'failed',
    error_message: LONG_ERROR,
    attempted_at: '2026-01-15T03:00:15Z',
  }
}

async function mockNotificationsApi(page: Page): Promise<void> {
  await page.route('**/api/notifications/channels', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([makeChannel()]),
    }),
  )
  await page.route('**/api/notifications/rules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/notifications/deliveries*', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([makeDelivery()]),
    }),
  )
  await page.route('**/api/notifications/push/vapid-key', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ public_key: '', configured: false }),
    }),
  )
  await page.route('**/api/repos', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/agents', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/schedules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
}

test('expands a delivery row and shows the full error and payload', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockNotificationsApi(page)

  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')

  await page.getByRole('button', { name: 'History' }).click()

  const deliveryRow = page.locator('.delivery-row')
  await expect(deliveryRow.first()).toBeVisible({ timeout: 10_000 })
  await expect(page.locator('.detail-row')).toHaveCount(0)

  await deliveryRow.first().click()

  const detailPanel = page.locator('.detail-panel')
  await expect(detailPanel).toBeVisible({ timeout: 10_000 })
  await expect(detailPanel).toContainText('Network is unreachable')
  await expect(detailPanel).toContainText('web-server-01')
  await expect(detailPanel).toContainText('daily-backup')

  // Collapses again on a second click.
  await deliveryRow.first().click()
  await expect(page.locator('.detail-row')).toHaveCount(0)
})

test('history table switches to a stacked card layout on narrow viewports', async ({
  page,
}: {
  page: Page
}) => {
  await page.setViewportSize({ width: 375, height: 800 })
  await loginAsAdmin(page)
  await mockNotificationsApi(page)

  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')
  await page.getByRole('button', { name: 'History' }).click()
  await expect(page.locator('.delivery-row').first()).toBeVisible({ timeout: 10_000 })

  const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth)
  const clientWidth = await page.evaluate(() => document.documentElement.clientWidth)
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth)
})
