// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'

type MockedApiClient = {
  get: ReturnType<typeof vi.fn>
  post: ReturnType<typeof vi.fn>
  put: ReturnType<typeof vi.fn>
  delete: ReturnType<typeof vi.fn>
}

const apiClient = vi.hoisted<MockedApiClient>(() => ({
  get: vi.fn(),
  post: vi.fn(),
  put: vi.fn(),
  delete: vi.fn(),
}))

vi.mock('./client', () => ({
  apiClient,
}))

import {
  createChannel,
  createRule,
  deleteChannel,
  deleteRule,
  getVapidPublicKey,
  listChannels,
  listDeliveries,
  listPushSubscriptions,
  listRules,
  saveVapidKeys,
  subscribePush,
  testChannel,
  unsubscribePush,
  validateSmtp,
  updateChannel,
} from './notifications'

describe('notifications api', () => {
  beforeEach(() => {
    apiClient.get.mockReset()
    apiClient.post.mockReset()
    apiClient.put.mockReset()
    apiClient.delete.mockReset()
  })

  it('lists channels', async () => {
    apiClient.get.mockResolvedValue({ data: [] })

    await listChannels()

    expect(apiClient.get).toHaveBeenCalledWith('/notifications/channels')
  })

  it('creates a channel', async () => {
    apiClient.post.mockResolvedValue({ data: {} })

    await createChannel({
      name: 'Ops email',
      channel_type: 'email',
      config: {
        smtp_host: 'smtp.example.com',
        smtp_port: 587,
        smtp_user: 'alerts',
        smtp_password: 'secret',
        from_address: 'alerts@example.com',
        to_addresses: ['ops@example.com'],
        security: 'starttls',
      },
      enabled: true,
    })

    expect(apiClient.post).toHaveBeenCalledWith('/notifications/channels', {
      name: 'Ops email',
      channel_type: 'email',
      config: {
        smtp_host: 'smtp.example.com',
        smtp_port: 587,
        smtp_user: 'alerts',
        smtp_password: 'secret',
        from_address: 'alerts@example.com',
        to_addresses: ['ops@example.com'],
        security: 'starttls',
      },
      enabled: true,
    })
  })

  it('updates a channel', async () => {
    apiClient.put.mockResolvedValue({ data: {} })

    await updateChannel(4, { enabled: false })

    expect(apiClient.put).toHaveBeenCalledWith('/notifications/channels/4', { enabled: false })
  })

  it('deletes a channel', async () => {
    apiClient.delete.mockResolvedValue({})

    await deleteChannel(4)

    expect(apiClient.delete).toHaveBeenCalledWith('/notifications/channels/4')
  })

  it('tests a channel', async () => {
    apiClient.post.mockResolvedValue({})

    await testChannel(4)

    expect(apiClient.post).toHaveBeenCalledWith('/notifications/channels/4/test')
  })

  it('lists rules', async () => {
    apiClient.get.mockResolvedValue({ data: [] })

    await listRules()

    expect(apiClient.get).toHaveBeenCalledWith('/notifications/rules')
  })

  it('creates a rule', async () => {
    apiClient.post.mockResolvedValue({ data: {} })

    await createRule({
      channel_id: 1,
      event_type: 'backup_failed',
      enabled: true,
    })

    expect(apiClient.post).toHaveBeenCalledWith('/notifications/rules', {
      channel_id: 1,
      event_type: 'backup_failed',
      enabled: true,
    })
  })

  it('deletes a rule', async () => {
    apiClient.delete.mockResolvedValue({})

    await deleteRule(9)

    expect(apiClient.delete).toHaveBeenCalledWith('/notifications/rules/9')
  })

  it('maps vapid key responses', async () => {
    apiClient.get.mockResolvedValue({ data: { public_key: 'abc', configured: true } })

    await expect(getVapidPublicKey()).resolves.toEqual({ key: 'abc', configured: true })
    expect(apiClient.get).toHaveBeenCalledWith('/notifications/push/vapid-key')
  })

  it('saves vapid keys', async () => {
    apiClient.put.mockResolvedValue({})

    await saveVapidKeys('pub', 'priv')

    expect(apiClient.put).toHaveBeenCalledWith('/notifications/push/vapid-key', {
      public_key: 'pub',
      private_key: 'priv',
    })
  })

  it('subscribes push notifications', async () => {
    apiClient.post.mockResolvedValue({})

    await subscribePush({ endpoint: 'https://push.example.com', keys: { p256dh: 'a', auth: 'b' } })

    expect(apiClient.post).toHaveBeenCalledWith('/notifications/push/subscribe', {
      endpoint: 'https://push.example.com',
      keys: { p256dh: 'a', auth: 'b' },
    })
  })

  it('unsubscribes push notifications', async () => {
    apiClient.post.mockResolvedValue({})

    await unsubscribePush('https://push.example.com')

    expect(apiClient.post).toHaveBeenCalledWith('/notifications/push/unsubscribe', {
      endpoint: 'https://push.example.com',
    })
  })

  it('lists push subscriptions', async () => {
    apiClient.get.mockResolvedValue({ data: [] })

    await listPushSubscriptions()

    expect(apiClient.get).toHaveBeenCalledWith('/notifications/push/subscriptions')
  })

  it('lists deliveries with and without limit', async () => {
    apiClient.get.mockResolvedValue({ data: [] })

    await listDeliveries()
    await listDeliveries(25)

    expect(apiClient.get).toHaveBeenNthCalledWith(1, '/notifications/deliveries', {
      params: undefined,
    })
    expect(apiClient.get).toHaveBeenNthCalledWith(2, '/notifications/deliveries', {
      params: { limit: 25 },
    })
  })

  it('validates smtp settings', async () => {
    apiClient.post.mockResolvedValue({})

    await validateSmtp({
      smtp_host: 'smtp.example.com',
      smtp_port: 587,
      smtp_user: 'alerts',
      smtp_password: 'secret',
      security: 'starttls',
    })

    expect(apiClient.post).toHaveBeenCalledWith('/notifications/validate-smtp', {
      smtp_host: 'smtp.example.com',
      smtp_port: 587,
      smtp_user: 'alerts',
      smtp_password: 'secret',
      security: 'starttls',
    })
  })
})
