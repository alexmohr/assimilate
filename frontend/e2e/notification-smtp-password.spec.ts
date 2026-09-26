// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockNotificationsApi, test } from './fixtures'
import type { Page } from '@playwright/test'

// Against the real demo server: the seeded email channel has a stored SMTP
// password, and the channel list must say so without ever carrying it.
test('the channel list never returns an SMTP password', async ({ page }: { page: Page }) => {
  await loginAsAdmin(page)

  const response = await page.request.get('/api/notifications/channels')
  expect(response.ok()).toBe(true)
  const body = await response.text()
  expect(body).not.toContain('demo-smtp-password')
  const channels = JSON.parse(body) as {
    name: string
    config: Record<string, unknown>
    has_password: boolean
  }[]

  for (const channel of channels) {
    expect(channel.config).not.toHaveProperty('smtp_password')
  }
  expect(channels.find((c) => c.name === 'Admin Email')?.has_password).toBe(true)
})

const EMAIL_CHANNEL = {
  id: 1,
  name: 'Ops Email',
  channel_type: 'email',
  config: {
    smtp_host: 'smtp.example.com',
    smtp_port: 587,
    smtp_user: 'alerts',
    from_address: 'alerts@example.com',
    to_addresses: ['ops@example.com'],
    security: 'starttls',
  },
  has_password: true,
  webhook_headers: [],
  enabled: true,
  scope: {},
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

test('editing an email channel keeps its saved password when the field is left blank', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockNotificationsApi(page, { channels: [EMAIL_CHANNEL], deliveries: [] })

  let validateBody: Record<string, unknown> | null = null
  await page.route('**/api/notifications/validate-smtp', (route) => {
    validateBody = route.request().postDataJSON() as Record<string, unknown>
    return route.fulfill({ status: 204 })
  })
  let savedBody: { config?: Record<string, unknown> } | null = null
  await page.route('**/api/notifications/channels/1', (route) => {
    savedBody = route.request().postDataJSON() as { config?: Record<string, unknown> }
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(EMAIL_CHANNEL),
    })
  })

  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')
  const card = page.locator('.channel-card').filter({ hasText: 'Ops Email' })
  await expect(card).toBeVisible({ timeout: 10_000 })
  await card.getByRole('button', { name: 'Edit', exact: true }).click()

  const dialog = page.locator('.modal-dialog')
  const password = dialog.getByTestId('smtp-password')
  await expect(password).toHaveValue('')
  await expect(password).toHaveAttribute('placeholder', /leave blank to keep it/)
  await expect(dialog).toContainText('A password is saved for this channel')

  await dialog.getByRole('button', { name: 'Save' }).click()
  await expect(dialog).toBeHidden()

  expect(validateBody).toMatchObject({ smtp_password: '', channel_id: 1 })
  expect(savedBody?.config).toMatchObject({ smtp_host: 'smtp.example.com', smtp_password: '' })
})
