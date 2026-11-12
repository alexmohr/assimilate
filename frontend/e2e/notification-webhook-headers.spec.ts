// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockNotificationsApi, openSettingsSection, test } from './fixtures'
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
  link: { target: 'automatic', url: null },
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
  // A channel row opens the channel's own page.
  const row = page.locator('.list-row[data-channel="Ops Webhook"]')
  await expect(row).toBeVisible({ timeout: 10_000 })
  await row.click()

  // The channel's page opens on its delivery settings, edited behind Edit.
  const settings = await openSettingsSection(page, 'Delivery')
  await settings.getByRole('button', { name: 'Edit' }).click()
  await expect(settings.getByTestId('webhook-header-name')).toHaveValue('Authorization')
  const value = settings.getByTestId('webhook-header-value')
  await expect(value).toHaveValue('')
  await expect(value).toHaveAttribute('placeholder', /leave blank to keep it/)

  // Pointing the URL at another host asks for the saved value again...
  const url = settings.getByPlaceholder('https://hooks.example.com/notify')
  await url.fill('https://hooks.attacker.example/notify')
  await expect(settings.getByTestId('webhook-header-reentry')).toContainText('Authorization')
  // ...and going back to the saved host does not.
  await url.fill('https://hooks.example.com/notify')
  await expect(settings.getByTestId('webhook-header-reentry')).toBeHidden()

  await settings.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(settings.getByRole('button', { name: 'Edit' })).toBeVisible()

  expect(savedBody?.config).toMatchObject({
    url: 'https://hooks.example.com/notify',
    headers: { Authorization: null },
  })
})
