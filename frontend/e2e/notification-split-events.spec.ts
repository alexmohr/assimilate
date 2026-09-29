// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockNotificationsApi, opsWebhookWithRules, test } from './fixtures'
import type { Page } from '@playwright/test'

test('a channel can follow file-changed warnings and abandoned catch-ups on their own', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockNotificationsApi(page, {
    ...opsWebhookWithRules(['backup_warning']),
    deliveries: [],
  })

  const created: string[] = []
  await page.route('**/api/notifications/rules', async (route) => {
    if (route.request().method() !== 'POST') {
      return route.fallback()
    }
    const body = route.request().postDataJSON() as { channel_id: number; event_type: string }
    created.push(body.event_type)
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        id: 10 + created.length,
        channel_id: body.channel_id,
        event_type: body.event_type,
        enabled: true,
        repo_id: null,
        agent_id: null,
        schedule_id: null,
      }),
    })
  })

  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')
  await page.getByTitle('Edit events').first().click()

  const modal = page.locator('.events-list')
  await expect(modal).toBeVisible({ timeout: 10_000 })

  // The two events sit beside the ones they were split out of, each with a
  // toggle of its own, so a general warning can stay on while file changes
  // are turned off (and the other way round).
  const fileChanged = modal.locator('.event-item', { hasText: 'Backup file changed' })
  const abandoned = modal.locator('.event-item', { hasText: 'Backup catch up abandoned' })
  await expect(fileChanged).toBeVisible()
  await expect(abandoned).toBeVisible()

  await fileChanged.getByRole('switch').click()
  await abandoned.getByRole('switch').click()

  await expect.poll(() => created).toEqual(['backup_file_changed', 'backup_catch_up_abandoned'])
  await expect(fileChanged.getByRole('switch')).toBeChecked()
  await expect(abandoned.getByRole('switch')).toBeChecked()
})
