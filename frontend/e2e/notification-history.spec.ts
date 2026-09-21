// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import {
  expect,
  loginAsAdmin,
  mockNotificationsApi,
  openNotificationHistory,
  test,
} from './fixtures'
import type { Page } from '@playwright/test'

const LONG_ERROR =
  'web push transport error: failed to connect to the server -> [7] Could not connect to server (raw TCP probe: [2001:4860:4802:36::39]:443 failed: Network is unreachable (os error 101); 216.239.36.55:443 connected)'

async function mockFailedWebPushDelivery(page: Page): Promise<void> {
  await mockNotificationsApi(page, {
    channels: [
      {
        id: 1,
        name: 'Ops Web Push',
        channel_type: 'web_push',
        config: { user_id: 1 },
        enabled: true,
        scope: {},
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
      },
    ],
    deliveries: [
      {
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
      },
    ],
  })
}

test('expands a delivery row and shows the full error and payload', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockFailedWebPushDelivery(page)

  const deliveryRow = await openNotificationHistory(page)
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
  await mockFailedWebPushDelivery(page)

  await openNotificationHistory(page)

  const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth)
  const clientWidth = await page.evaluate(() => document.documentElement.clientWidth)
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth)
})
