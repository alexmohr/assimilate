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

const REASON = "the host for repository 'database-hourly' did not answer SSH"

test('a backup skipped because its repository host was offline reaches the history', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockNotificationsApi(page, {
    channels: [
      {
        id: 1,
        name: 'Ops Webhook',
        channel_type: 'webhook',
        config: { url: 'https://hooks.example.com/assimilate' },
        enabled: true,
        scope: {},
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
      },
    ],
    rules: [
      {
        id: 1,
        channel_id: 1,
        event_type: 'backup_skipped_repo_offline',
        enabled: true,
        repo_id: null,
        agent_id: null,
        schedule_id: null,
      },
    ],
    deliveries: [
      {
        id: 1,
        channel_id: 1,
        event_type: 'backup_skipped_repo_offline',
        payload: {
          event_type: 'backup_skipped_repo_offline',
          hostname: 'db-server-01',
          repo_name: 'database-hourly',
          schedule_name: 'Hourly database backup',
          status: 'skipped',
          error_message: REASON,
          timestamp: '2026-01-15T08:00:00Z',
        },
        status: 'sent',
        error_message: null,
        attempted_at: '2026-01-15T08:00:02Z',
      },
    ],
  })

  const deliveryRow = await openNotificationHistory(page)
  await expect(deliveryRow.first()).toContainText('Backup skipped repo offline')

  await deliveryRow.first().click()

  // Both skips share one "Backup skipped" label, so the payload is what has to
  // say which host was the one that wasn't there.
  const detailPanel = page.locator('.detail-panel')
  await expect(detailPanel).toBeVisible({ timeout: 10_000 })
  await expect(detailPanel).toContainText(REASON)
  await expect(detailPanel).toContainText('database-hourly')
  await expect(detailPanel).toContainText('db-server-01')
})
