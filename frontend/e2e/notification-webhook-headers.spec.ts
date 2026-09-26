// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockNotificationsApi, test } from './fixtures'
import type { Page } from '@playwright/test'

// Against the real demo server: the seeded webhook channel has a saved
// `Authorization` header, and the channel list must name it without ever
// carrying its value.
test('the channel list never returns a webhook header value', async ({ page }: { page: Page }) => {
  await loginAsAdmin(page)

  const response = await page.request.get('/api/notifications/channels')
  expect(response.ok()).toBe(true)
  const body = await response.text()
  expect(body).not.toContain('demo-token')
  const channels = JSON.parse(body) as {
    name: string
    config: Record<string, unknown>
    webhook_headers: { name: string; has_value: boolean }[]
  }[]

  for (const channel of channels) {
    expect(channel.config).not.toHaveProperty('headers')
  }
  expect(channels.find((c) => c.name === 'Ops Webhook')?.webhook_headers).toEqual([
    { name: 'Authorization', has_value: true },
  ])
})

const WEBHOOK_CHANNEL = {
  id: 1,
  name: 'Ops Webhook',
  channel_type: 'webhook',
  config: { url: 'https://hooks.example.com/notify' },
  has_password: false,
  webhook_headers: [{ name: 'Authorization', has_value: true }],
  enabled: true,
  scope: {},
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

test('editing a webhook channel keeps its saved header value when left blank', async ({
  page,
}: {
  page: Page
}) => {
  await loginAsAdmin(page)
  await mockNotificationsApi(page, { channels: [WEBHOOK_CHANNEL], deliveries: [] })

  let savedBody: { config?: Record<string, unknown> } | null = null
  await page.route('**/api/notifications/channels/1', (route) => {
    savedBody = route.request().postDataJSON() as { config?: Record<string, unknown> }
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(WEBHOOK_CHANNEL),
    })
  })

  await page.goto('/notifications')
  await page.waitForLoadState('networkidle')
  const card = page.locator('.channel-card').filter({ hasText: 'Ops Webhook' })
  await expect(card).toBeVisible({ timeout: 10_000 })
  await card.getByRole('button', { name: 'Edit', exact: true }).click()

  const dialog = page.locator('.modal-dialog')
  await expect(dialog.getByTestId('webhook-header-name')).toHaveValue('Authorization')
  const value = dialog.getByTestId('webhook-header-value')
  await expect(value).toHaveValue('')
  await expect(value).toHaveAttribute('placeholder', /leave blank to keep it/)

  // Pointing the URL at another host asks for the saved value again...
  const url = dialog.getByPlaceholder('https://hooks.example.com/notify')
  await url.fill('https://hooks.attacker.example/notify')
  await expect(dialog.getByTestId('webhook-header-reentry')).toContainText('Authorization')
  // ...and going back to the saved host does not.
  await url.fill('https://hooks.example.com/notify')
  await expect(dialog.getByTestId('webhook-header-reentry')).toBeHidden()

  await dialog.getByRole('button', { name: 'Save' }).click()
  await expect(dialog).toBeHidden()

  expect(savedBody?.config).toMatchObject({
    url: 'https://hooks.example.com/notify',
    headers: { Authorization: null },
  })
})
