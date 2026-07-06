// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import NotificationsView from './NotificationsView.vue'

vi.mock('../api/notifications', () => ({
  listChannels: vi.fn(),
  listRules: vi.fn(),
  listDeliveries: vi.fn(),
  getVapidPublicKey: vi.fn(),
  createChannel: vi.fn(),
  updateChannel: vi.fn(),
  deleteChannel: vi.fn(),
  testChannel: vi.fn(),
  createRule: vi.fn(),
  deleteRule: vi.fn(),
  subscribePush: vi.fn(),
  validateSmtp: vi.fn(),
}))

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: vi.fn(),
  }),
}))

vi.mock('../composables/useEscapeKey', () => ({
  useEscapeKey: vi.fn(),
}))

vi.mock('../utils/logger', () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
    info: vi.fn(),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback?: string) => fallback ?? 'Unknown error',
  extractBlobError: async (_e: unknown, fallback?: string): Promise<string> =>
    fallback ?? 'Unknown error',
}))

import { listChannels, listRules, listDeliveries, getVapidPublicKey } from '../api/notifications'
import { apiClient } from '../api/client'

const mockListChannels = vi.mocked(listChannels)
const mockListRules = vi.mocked(listRules)
const mockListDeliveries = vi.mocked(listDeliveries)
const mockGetVapidPublicKey = vi.mocked(getVapidPublicKey)
const mockApiGet = vi.mocked(apiClient.get)

import type { NotificationChannel, NotificationRule } from '../types/notifications'
import type { EmailConfig, WebhookConfig } from '../types/notifications'

const WEBHOOK_CHANNEL: NotificationChannel = {
  id: 1,
  name: 'Ops Webhook',
  channel_type: 'webhook',
  config: { url: 'https://hooks.example.com/notify' } as WebhookConfig,
  enabled: true,
  scope: {},
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

const EMAIL_CHANNEL: NotificationChannel = {
  id: 2,
  name: 'Ops Email',
  channel_type: 'email',
  config: {
    smtp_host: 'smtp.example.com',
    smtp_port: 587,
    smtp_user: 'user',
    smtp_password: 'pass',
    from_address: 'noreply@example.com',
    to_addresses: ['admin@example.com'],
    security: 'starttls',
  } as EmailConfig,
  enabled: true,
  scope: {},
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

const MOCK_RULES: NotificationRule[] = [
  {
    id: 1,
    channel_id: 1,
    event_type: 'backup_failed',
    repo_id: null,
    agent_id: null,
    enabled: true,
  },
]

function setupDefaultMocks(): void {
  mockListChannels.mockResolvedValue([WEBHOOK_CHANNEL, EMAIL_CHANNEL])
  mockListRules.mockResolvedValue(MOCK_RULES)
  mockListDeliveries.mockResolvedValue([])
  mockGetVapidPublicKey.mockResolvedValue({ key: '', configured: false })
  mockApiGet.mockResolvedValue({ data: [] })
}

describe('NotificationsView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders page title', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('Notifications')
  })

  it('renders Channels and History tabs', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('Channels')
    expect(wrapper.text()).toContain('History')
  })

  it('renders webhook channel card', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('Ops Webhook')
  })

  it('renders email channel card', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('Ops Email')
  })

  it('renders channel type badges', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('Webhook')
    expect(wrapper.text()).toContain('Email')
  })

  it('renders Test and Edit buttons per channel', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const buttons = wrapper.findAll('button').map((b) => b.text())
    expect(buttons.filter((t) => t === 'Test').length).toBeGreaterThanOrEqual(2)
    expect(buttons.filter((t) => t === 'Edit').length).toBeGreaterThanOrEqual(2)
  })

  it('renders New button for adding a channel', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('New')
  })

  it('shows empty state when no channels exist', async () => {
    mockListChannels.mockResolvedValue([])
    mockListRules.mockResolvedValue([])
    mockListDeliveries.mockResolvedValue([])
    mockGetVapidPublicKey.mockResolvedValue({ key: '', configured: false })
    mockApiGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('No notification channels')
  })

  it('shows empty delivery history message on History tab', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const historyTab = wrapper.findAll('button').find((b) => b.text().includes('History'))
    await historyTab!.trigger('click')
    expect(wrapper.text()).toContain('No delivery history yet')
  })

  it('shows Add Channel wizard when New is clicked', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const newBtn = wrapper.findAll('button').find((b) => b.text().includes('New'))
    await newBtn!.trigger('click')
    await flushPromises()
    expect(document.body.textContent).toContain('New Channel')
  })

  it('switches to webhook config in add wizard', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const newBtn = wrapper.findAll('button').find((b) => b.text().includes('New'))
    await newBtn!.trigger('click')
    await flushPromises()

    // Switch channel type to webhook - exercises resetAddChannelConfig and createWebhookConfig
    const typeSelect = document.body.querySelector('select')
    typeSelect!.value = 'webhook'
    typeSelect!.dispatchEvent(new Event('change', { bubbles: true }))
    await flushPromises()

    // Webhook URL input should render - exercises addChannelWebhookCfg and isWebhookConfig
    expect(document.body.textContent).toContain('URL')
  })

  it('opens edit dialog for webhook channel', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()

    // Find the Edit button for the webhook channel (first in list)
    const editBtns = wrapper.findAll('button').filter((b) => b.text() === 'Edit')
    await editBtns[0].trigger('click')
    await flushPromises()

    // Edit dialog should show webhook URL field - exercises editChannelWebhookCfg and isWebhookConfig
    expect(document.body.textContent).toContain('URL')
  })
})
