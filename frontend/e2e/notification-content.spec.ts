// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

function makeChannel(overrides: Record<string, unknown> = {}): object {
  return {
    id: 1,
    name: 'Ops Email',
    channel_type: 'email',
    config: {
      smtp_host: 'smtp.example.com',
      smtp_port: 587,
      smtp_user: 'user',
      smtp_password: '',
      from_address: 'alerts@example.com',
      to_addresses: ['ops@example.com'],
      security: 'starttls',
    },
    enabled: true,
    scope: {},
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  }
}

async function mockNotificationsApi(page: Page): Promise<void> {
  await page.route('**/api/notifications/channels', (route) => {
    if (route.request().method() === 'GET') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([makeChannel()]),
      })
    }
    return route.continue()
  })
  await page.route('**/api/notifications/channels/1', (route) => {
    if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON() as { config?: Record<string, unknown> }
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(makeChannel(body.config ? { config: body.config } : {})),
      })
    }
    return route.continue()
  })
  await page.route('**/api/notifications/rules', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/notifications/deliveries*', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '[]' }),
  )
  await page.route('**/api/notifications/push/vapid-key', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ key: '', configured: false }),
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

test('edits and saves a channel-specific notification content template', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockNotificationsApi(page)

  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')

  const channelCard = page.locator('.channel-card').filter({ hasText: 'Ops Email' })
  await expect(channelCard).toBeVisible({ timeout: 10_000 })

  // The default template is pre-filled from the moment the panel opens - no "customize"
  // toggle to find first.
  await channelCard.getByRole('button', { name: 'Edit content' }).click()
  const titleField = channelCard.getByLabel('Title')
  await expect(titleField).toHaveValue('{{event}}: {{host}} / {{repository}}')

  // The live preview shows the deduplicated size on a successful backup by default.
  await expect(channelCard).toContainText('Dedup size shown by default')
  await expect(channelCard).toContainText('MiB new')

  // Clicking a variable chip inserts it at the cursor in the last-focused field.
  await titleField.click()
  await titleField.press('End')
  await channelCard.getByRole('button', { name: '{{archive}}', exact: true }).click()
  await expect(titleField).toHaveValue('{{event}}: {{host}} / {{repository}}{{archive}}')

  // Editing and saving only affects this channel's own template.
  await titleField.fill('Backup report for {{host}}')
  const saveButton = channelCard.getByRole('button', { name: 'Save content' })
  await expect(saveButton).toBeEnabled()
  await saveButton.click()

  await expect(channelCard.getByRole('button', { name: /Saving/ })).toHaveCount(0)
  await expect(saveButton).toBeDisabled()
})
