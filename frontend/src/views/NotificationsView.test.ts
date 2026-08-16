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
    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.find('.empty-title').text()).toBe('No deliveries yet')
  })

  it('expands a delivery row to reveal the full error and payload', async () => {
    mockListChannels.mockResolvedValue([WEBHOOK_CHANNEL, EMAIL_CHANNEL])
    mockListRules.mockResolvedValue(MOCK_RULES)
    mockListDeliveries.mockResolvedValue([
      {
        id: 1,
        channel_id: WEBHOOK_CHANNEL.id,
        event_type: 'backup_failed',
        payload: { hostname: 'web-01', repo_name: 'daily-backup' },
        status: 'failed',
        error_message: 'connection refused',
        attempted_at: '2026-01-15T03:00:12Z',
      },
    ])
    mockGetVapidPublicKey.mockResolvedValue({ key: '', configured: false })
    mockApiGet.mockResolvedValue({ data: [] })

    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const historyTab = wrapper.findAll('button').find((b) => b.text().includes('History'))
    await historyTab!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).not.toContain('web-01')

    const deliveryRow = wrapper.find('.delivery-row')
    expect(deliveryRow.exists()).toBe(true)
    await deliveryRow.trigger('click')
    await flushPromises()

    expect(wrapper.find('.detail-row').exists()).toBe(true)
    expect(wrapper.text()).toContain('web-01')
    expect(wrapper.text()).toContain('daily-backup')

    await deliveryRow.trigger('click')
    await flushPromises()
    expect(wrapper.find('.detail-row').exists()).toBe(false)
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

  describe('add channel wizard', () => {
    function dialogButton(label: string): HTMLButtonElement {
      const match = [...document.body.querySelectorAll<HTMLButtonElement>('button')].find(
        (b) => b.textContent?.trim() === label,
      )
      if (!match) throw new Error(`no button labelled "${label}"`)
      return match
    }

    function fieldByLabel(label: string): HTMLInputElement {
      const wrap = [...document.body.querySelectorAll('.field')].find((f) =>
        f.querySelector('.field-label')?.textContent?.includes(label),
      )
      const control = wrap?.querySelector('input, select')
      if (!control) throw new Error(`no field labelled "${label}"`)
      return control as HTMLInputElement
    }

    async function setByLabel(label: string, value: string): Promise<void> {
      const control = fieldByLabel(label)
      control.value = value
      control.dispatchEvent(new Event('input'))
      control.dispatchEvent(new Event('change'))
      await flushPromises()
    }

    /** Fills step 1 with a valid email channel and advances to step 2. */
    async function fillStepOne(): Promise<void> {
      await setByLabel('Name', 'Ops Alerts')
      await setByLabel('SMTP Host', 'smtp.example.com')
      await setByLabel('From Address', 'noreply@example.com')
      await setByLabel('To Addresses', 'admin@example.com')
    }

    /**
     * The view itself is not attached to the document - only the teleported
     * dialog is - so the trigger is found through the wrapper while the
     * wizard's own controls are found on document.body.
     */
    async function openWizard() {
      setupDefaultMocks()
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      await openFromHeader(wrapper)
      return wrapper
    }

    async function openFromHeader(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
      const trigger = wrapper.findAll('button').find((b) => b.text().includes('New'))
      if (!trigger) throw new Error('no New button in the page header')
      await trigger.trigger('click')
      await flushPromises()
    }

    it('opens on step 1 of 3', async () => {
      await openWizard()
      expect(document.body.textContent).toContain('Step 1 of 3')
    })

    // Step 1 is the only step with required fields, so it is the only one
    // that may block Next; steps 2 and 3 are both legitimately skippable.
    it('will not advance past step 1 until the channel is actually configured', async () => {
      await openWizard()
      expect(dialogButton('Next').disabled).toBe(true)

      await fillStepOne()
      expect(dialogButton('Next').disabled).toBe(false)
    })

    it('walks forward and back through the three steps', async () => {
      await openWizard()
      await fillStepOne()

      dialogButton('Next').click()
      await flushPromises()
      expect(document.body.textContent).toContain('Step 2 of 3')
      expect(document.body.textContent).toContain('Select which events')

      dialogButton('Next').click()
      await flushPromises()
      expect(document.body.textContent).toContain('Step 3 of 3')

      dialogButton('Back').click()
      await flushPromises()
      expect(document.body.textContent).toContain('Step 2 of 3')
    })

    it('offers Cancel on the first step and Back thereafter', async () => {
      await openWizard()
      await fillStepOne()
      expect(() => dialogButton('Cancel')).not.toThrow()

      dialogButton('Next').click()
      await flushPromises()

      expect(() => dialogButton('Cancel')).toThrow()
      expect(() => dialogButton('Back')).not.toThrow()
    })

    it('closes without creating anything on Cancel', async () => {
      const { createChannel } = await import('../api/notifications')
      await openWizard()
      dialogButton('Cancel').click()
      await flushPromises()

      expect(document.body.textContent).not.toContain('Step 1 of 3')
      expect(vi.mocked(createChannel)).not.toHaveBeenCalled()
    })

    it('lists every event type as a toggle on step 2', async () => {
      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()

      expect(document.body.querySelectorAll('.event-item')).toHaveLength(7)
    })

    it('creates the channel and one rule per selected event', async () => {
      const { createChannel, createRule, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(createChannel).mockResolvedValue({
        id: 42,
        name: 'Ops Alerts',
        channel_type: 'email',
        config: {},
        enabled: true,
        scope: {},
      } as never)
      vi.mocked(createRule).mockImplementation(
        async (req: unknown) => ({ id: 1, ...(req as object) }) as never,
      )

      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()

      const toggles = [...document.body.querySelectorAll('.event-item')]
      ;(toggles[0].querySelector('input, button') as HTMLElement | null)?.click()
      ;(toggles[2].querySelector('input, button') as HTMLElement | null)?.click()
      await flushPromises()

      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Create').click()
      await flushPromises()

      expect(vi.mocked(createChannel)).toHaveBeenCalledTimes(1)
      // One rule per event picked on step 2, each bound to the new channel.
      expect(vi.mocked(createRule)).toHaveBeenCalledTimes(2)
      expect(vi.mocked(createRule).mock.calls.every((c) => c[0].channel_id === 42)).toBe(true)
      expect(vi.mocked(createRule).mock.calls.map((c) => c[0].event_type)).toEqual([
        'backup_success',
        'backup_failed',
      ])
    })

    // A channel whose SMTP credentials cannot log in would silently swallow
    // every notification, so the wizard verifies before it stores anything.
    it('refuses to create an email channel whose SMTP login fails', async () => {
      const { createChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockRejectedValue(new Error('535 auth failed'))

      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Create').click()
      await flushPromises()

      // Assert the check actually ran: this test passed for the wrong reason
      // while the gate was reaching through a ref that no longer existed, so
      // nothing was ever verified and every channel was rejected.
      expect(vi.mocked(validateSmtp)).toHaveBeenCalledTimes(1)
      expect(vi.mocked(createChannel)).not.toHaveBeenCalled()
      expect(document.body.querySelector('.form-error')).not.toBeNull()
    })

    /**
     * The SMTP fields live on step 1, so by the time Create is pressed on
     * step 3 the fields component is unmounted. Gating the save on a ref to
     * it meant the check silently never ran and every email channel - the
     * default type - was rejected with "SMTP validation failed" no matter how
     * good the credentials were. The check has to survive its fields leaving
     * the screen.
     */
    it('validates SMTP from step 3, where the fields are no longer rendered', async () => {
      const { createChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(createChannel).mockResolvedValue({ id: 42, scope: {} } as never)

      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()

      expect(
        [...document.body.querySelectorAll('.field-label')].some((l) =>
          l.textContent?.includes('SMTP Host'),
        ),
      ).toBe(false)

      dialogButton('Create').click()
      await flushPromises()

      expect(vi.mocked(validateSmtp)).toHaveBeenCalledWith(
        expect.objectContaining({ smtp_host: 'smtp.example.com' }),
      )
      expect(vi.mocked(createChannel)).toHaveBeenCalledTimes(1)
    })

    it('reports a create failure without closing the wizard', async () => {
      const { createChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(createChannel).mockRejectedValue(new Error('duplicate name'))

      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Create').click()
      await flushPromises()

      expect(document.body.textContent).toContain('Step 3 of 3')
      expect(document.body.querySelector('.form-error')).not.toBeNull()
    })

    it('starts from a clean form when reopened after a cancel', async () => {
      const wrapper = await openWizard()
      await setByLabel('Name', 'discarded')
      dialogButton('Cancel').click()
      await flushPromises()

      await openFromHeader(wrapper)

      expect((fieldByLabel('Name') as HTMLInputElement).value).toBe('')
      expect(document.body.textContent).toContain('Step 1 of 3')
    })
  })
})
