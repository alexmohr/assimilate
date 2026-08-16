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

/**
 * Every dialog in this view teleports, so its controls are queried off the
 * document body rather than through the wrapper. Shared by the wizard, edit
 * and scope/delete suites below.
 */
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

  describe('edit channel', () => {
    /** Opens the edit dialog for a channel by its card position. */
    async function openEdit(index: number) {
      setupDefaultMocks()
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      const editBtns = wrapper.findAll('button').filter((b) => b.text() === 'Edit')
      await editBtns[index].trigger('click')
      await flushPromises()
      return wrapper
    }

    it('prefills the dialog from the channel being edited', async () => {
      await openEdit(1)
      expect((fieldByLabel('Name') as HTMLInputElement).value).toBe('Ops Email')
      expect((fieldByLabel('SMTP Host') as HTMLInputElement).value).toBe('smtp.example.com')
    })

    // The wire type is a list; the field is one comma-separated string.
    it('joins the recipient list into the single address field', async () => {
      await openEdit(1)
      expect((fieldByLabel('To Addresses') as HTMLInputElement).value).toBe('admin@example.com')
    })

    it('shows the webhook URL instead for a webhook channel', async () => {
      await openEdit(0)
      expect((fieldByLabel('URL') as HTMLInputElement).value).toBe(
        'https://hooks.example.com/notify',
      )
      expect(() => fieldByLabel('SMTP Host')).toThrow()
    })

    it('saves the edited channel and splits the recipients back into a list', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(updateChannel).mockResolvedValue({ ...EMAIL_CHANNEL, name: 'Renamed' } as never)

      await openEdit(1)
      await setByLabel('Name', 'Renamed')
      await setByLabel('To Addresses', 'a@example.com, b@example.com ,')
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(
        2,
        expect.objectContaining({
          name: 'Renamed',
          config: expect.objectContaining({
            to_addresses: ['a@example.com', 'b@example.com'],
          }),
        }),
      )
    })

    it('checks SMTP before saving an email channel', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockRejectedValue(new Error('535 auth failed'))

      await openEdit(1)
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(validateSmtp)).toHaveBeenCalledTimes(1)
      expect(vi.mocked(updateChannel)).not.toHaveBeenCalled()
      expect(document.body.querySelector('.form-error')).not.toBeNull()
    })

    // A webhook has no credentials to check, so it must not be sent through
    // the SMTP gate - doing so would make webhooks unsaveable.
    it('does not run the SMTP check for a webhook channel', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(updateChannel).mockResolvedValue(WEBHOOK_CHANNEL as never)

      await openEdit(0)
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(validateSmtp)).not.toHaveBeenCalled()
      expect(vi.mocked(updateChannel)).toHaveBeenCalledTimes(1)
    })

    it('reports a save failure without closing the dialog', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(updateChannel).mockRejectedValue(new Error('conflict'))

      await openEdit(1)
      dialogButton('Save').click()
      await flushPromises()

      expect(document.body.querySelector('.form-error')).not.toBeNull()
      expect(document.body.querySelector('.modal-dialog')).not.toBeNull()
    })

    it('closes without saving on Cancel', async () => {
      const { updateChannel } = await import('../api/notifications')
      await openEdit(1)
      dialogButton('Cancel').click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).not.toHaveBeenCalled()
      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })
  })

  describe('channel scope and deletion', () => {
    const SCOPE_REPOS = [{ id: 7, name: 'server-daily' }]
    const SCOPE_AGENTS = [{ id: 3, hostname: 'web-01', display_name: null }]
    const SCOPE_SCHEDULES = [{ id: 5, agent_id: 3, repo_id: 7 }]

    function scopedMocks(): void {
      setupDefaultMocks()
      mockApiGet.mockImplementation((url: string) => {
        if (url === '/repos') return Promise.resolve({ data: SCOPE_REPOS })
        if (url === '/agents') return Promise.resolve({ data: SCOPE_AGENTS })
        if (url === '/schedules') return Promise.resolve({ data: SCOPE_SCHEDULES })
        return Promise.resolve({ data: [] })
      })
    }

    async function render() {
      scopedMocks()
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      return wrapper
    }

    async function clickByTitle(wrapper: Awaited<ReturnType<typeof render>>, title: string) {
      const btn = wrapper.findAll('button').find((b) => b.attributes('title') === title)
      if (!btn) throw new Error(`no button titled "${title}"`)
      await btn.trigger('click')
      await flushPromises()
    }

    it('offers the loaded repositories, agents and schedules as wizard scope', async () => {
      const wrapper = await render()
      const trigger = wrapper.findAll('button').find((b) => b.text().includes('New'))!
      await trigger.trigger('click')

      const set = async (label: string, v: string) => {
        const wrap = [...document.body.querySelectorAll('.field')].find((f) =>
          f.querySelector('.field-label')?.textContent?.includes(label),
        )
        const c = wrap!.querySelector('input, select') as HTMLInputElement
        c.value = v
        c.dispatchEvent(new Event('input'))
        c.dispatchEvent(new Event('change'))
        await flushPromises()
      }
      await set('Name', 'Scoped')
      await set('SMTP Host', 'smtp.example.com')
      await set('From Address', 'noreply@example.com')
      await set('To Addresses', 'admin@example.com')

      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()

      const text = document.body.textContent ?? ''
      expect(text).toContain('server-daily')
      expect(text).toContain('web-01')
      expect(document.body.querySelectorAll('.scope-item').length).toBeGreaterThanOrEqual(3)
    })

    it('narrows the scope list as you search', async () => {
      const wrapper = await render()
      await clickByTitle(wrapper, 'Edit scope')

      const before = document.body.querySelectorAll('.scope-item').length
      const search = document.body.querySelector<HTMLInputElement>('.scope-search')!
      search.value = 'server-daily'
      search.dispatchEvent(new Event('input'))
      await flushPromises()

      const after = document.body.querySelectorAll('.scope-item').length
      expect(after).toBeLessThan(before)
      expect(document.body.textContent).toContain('server-daily')
    })

    it('opens the per-channel events editor', async () => {
      const wrapper = await render()
      await clickByTitle(wrapper, 'Edit events')
      expect(document.body.querySelector('.modal-dialog')).not.toBeNull()
      expect(document.body.textContent).toContain('Backup')
    })

    it('names the channel it is about to delete', async () => {
      const wrapper = await render()
      const del = wrapper.findAll('button.btn-danger-text')[0]
      await del.trigger('click')
      await flushPromises()
      expect(document.body.textContent).toContain('Ops Webhook')
    })

    it('deletes the channel on confirmation and drops its rules', async () => {
      const { deleteChannel } = await import('../api/notifications')
      vi.mocked(deleteChannel).mockResolvedValue(undefined as never)

      const wrapper = await render()
      const del = wrapper.findAll('button.btn-danger-text')[0]
      await del.trigger('click')
      await flushPromises()
      dialogButton('Delete').click()
      await flushPromises()

      expect(vi.mocked(deleteChannel)).toHaveBeenCalledWith(1)
      expect(wrapper.text()).not.toContain('Ops Webhook')
    })

    it('keeps the channel when the delete is cancelled', async () => {
      const { deleteChannel } = await import('../api/notifications')
      const wrapper = await render()
      const del = wrapper.findAll('button.btn-danger-text')[0]
      await del.trigger('click')
      await flushPromises()
      dialogButton('Cancel').click()
      await flushPromises()

      expect(vi.mocked(deleteChannel)).not.toHaveBeenCalled()
      expect(wrapper.text()).toContain('Ops Webhook')
    })
  })

  describe('scope and event toggles', () => {
    const SCOPE_REPOS = [{ id: 7, name: 'server-daily' }]
    const SCOPE_AGENTS = [{ id: 3, hostname: 'web-01', display_name: null }]
    const SCOPE_SCHEDULES = [{ id: 5, agent_id: 3, repo_id: 7 }]

    async function render() {
      setupDefaultMocks()
      mockApiGet.mockImplementation((url: string) => {
        if (url === '/repos') return Promise.resolve({ data: SCOPE_REPOS })
        if (url === '/agents') return Promise.resolve({ data: SCOPE_AGENTS })
        if (url === '/schedules') return Promise.resolve({ data: SCOPE_SCHEDULES })
        return Promise.resolve({ data: [] })
      })
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      return wrapper
    }

    async function clickByTitle(wrapper: ReturnType<typeof renderWithPlugins>, title: string) {
      const btn = wrapper.findAll('button').find((b) => b.attributes('title') === title)
      if (!btn) throw new Error(`no button titled "${title}"`)
      await btn.trigger('click')
      await flushPromises()
    }

    async function fillStepOne(): Promise<void> {
      await setByLabel('Name', 'Scoped')
      await setByLabel('SMTP Host', 'smtp.example.com')
      await setByLabel('From Address', 'noreply@example.com')
      await setByLabel('To Addresses', 'admin@example.com')
    }

    async function openWizard(wrapper: ReturnType<typeof renderWithPlugins>) {
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('New'))!
        .trigger('click')
      await flushPromises()
    }

    // Scope is optional and additive: ticking a repository must narrow the
    // channel to it without dropping the agent or schedule selections.
    it('carries every scope kind through to the created channel', async () => {
      const { createChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(createChannel).mockResolvedValue({ id: 42, scope: {} } as never)

      const wrapper = await render()
      await openWizard(wrapper)
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()

      const boxes = document.body.querySelectorAll<HTMLInputElement>(
        '.scope-item input[type="checkbox"]',
      )
      expect(boxes.length).toBe(3)
      for (const box of boxes) box.click()
      await flushPromises()

      dialogButton('Create').click()
      await flushPromises()

      expect(vi.mocked(createChannel)).toHaveBeenCalledWith(
        expect.objectContaining({
          scope: { repo_ids: [7], agent_ids: [3], schedule_ids: [5] },
        }),
      )
    })

    it('creates a disabled channel when the enable switch is turned off', async () => {
      const { createChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(createChannel).mockResolvedValue({ id: 42, scope: {} } as never)

      const wrapper = await render()
      await openWizard(wrapper)
      await fillStepOne()

      // The enable switch is the only ToggleSwitch on step 1 of the wizard.
      const toggle = wrapper
        .findAllComponents({ name: 'ToggleSwitch' })
        .find((t) => t.text().includes('Enable immediately'))
      expect(toggle).toBeDefined()
      await toggle!.vm.$emit('update:modelValue', false)
      await flushPromises()

      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Create').click()
      await flushPromises()

      expect(vi.mocked(createChannel)).toHaveBeenCalledWith(
        expect.objectContaining({ enabled: false }),
      )
    })

    it('filters the wizard scope list as you search', async () => {
      const wrapper = await render()
      await openWizard(wrapper)
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()

      const before = document.body.querySelectorAll('.scope-item').length
      const search = document.body.querySelector<HTMLInputElement>('.scope-search')!
      search.value = 'server-daily'
      search.dispatchEvent(new Event('input'))
      await flushPromises()

      expect(document.body.querySelectorAll('.scope-item').length).toBeLessThan(before)
    })

    // A rule is a row, not a flag: enabling an event creates one and
    // disabling it deletes the existing row.
    it('creates a rule for an event that had none', async () => {
      const { createRule } = await import('../api/notifications')
      vi.mocked(createRule).mockResolvedValue({ id: 99 } as never)

      const wrapper = await render()
      await clickByTitle(wrapper, 'Edit events')

      const items = document.body.querySelectorAll('.event-item')
      expect(items.length).toBeGreaterThan(0)
      ;(items[0].querySelector('input, button') as HTMLElement | null)?.click()
      await flushPromises()

      expect(vi.mocked(createRule)).toHaveBeenCalled()
    })

    it('deletes the rule when its event is switched off', async () => {
      const { deleteRule } = await import('../api/notifications')
      vi.mocked(deleteRule).mockResolvedValue(undefined as never)

      const wrapper = await render()
      await clickByTitle(wrapper, 'Edit events')

      // MOCK_RULES already has backup_failed enabled for this channel.
      const item = [...document.body.querySelectorAll('.event-item')].find((el) =>
        el.textContent?.includes('Failed'),
      )
      expect(item).toBeDefined()
      ;(item!.querySelector('input, button') as HTMLElement | null)?.click()
      await flushPromises()

      expect(vi.mocked(deleteRule)).toHaveBeenCalledWith(1)
    })

    it('narrows an existing channel from the scope editor', async () => {
      const { updateChannel } = await import('../api/notifications')
      vi.mocked(updateChannel).mockResolvedValue({ id: 1, scope: { repo_ids: [7] } } as never)

      const wrapper = await render()
      await clickByTitle(wrapper, 'Edit scope')

      const box = document.body.querySelector<HTMLInputElement>(
        '.scope-item input[type="checkbox"]',
      )
      expect(box).not.toBeNull()
      box!.click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(
        1,
        expect.objectContaining({ scope: expect.objectContaining({ repo_ids: [7] }) }),
      )
    })

    it('narrows an existing channel by agent and schedule too', async () => {
      const { updateChannel } = await import('../api/notifications')
      vi.mocked(updateChannel).mockImplementation(
        async (id: number, body: { scope?: unknown }) => ({ id, scope: body.scope ?? {} }) as never,
      )

      const wrapper = await render()
      await clickByTitle(wrapper, 'Edit scope')

      const boxes = [
        ...document.body.querySelectorAll<HTMLInputElement>('.scope-item input[type="checkbox"]'),
      ]
      expect(boxes).toHaveLength(3)
      for (const box of boxes) {
        box.click()
        await flushPromises()
      }

      const kinds = vi
        .mocked(updateChannel)
        .mock.calls.map((c) => Object.keys((c[1] as { scope: object }).scope).at(-1))
      expect(kinds).toEqual(['repo_ids', 'agent_ids', 'schedule_ids'])
    })

    it('disables an existing channel from the edit dialog', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(updateChannel).mockResolvedValue(EMAIL_CHANNEL as never)

      const wrapper = await render()
      await wrapper
        .findAll('button')
        .filter((b) => b.text() === 'Edit')[1]
        .trigger('click')
      await flushPromises()

      const toggle = wrapper
        .findAllComponents({ name: 'ToggleSwitch' })
        .find((t) => t.text().includes('Enabled'))
      expect(toggle).toBeDefined()
      await toggle!.vm.$emit('update:modelValue', false)
      await flushPromises()

      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(
        2,
        expect.objectContaining({ enabled: false }),
      )
    })

    it.each([['Edit events'], ['Edit scope']])('closes the %s editor again', async (title) => {
      const wrapper = await render()
      await clickByTitle(wrapper, title)
      expect(document.body.querySelector('.modal-dialog')).not.toBeNull()

      dialogButton('Done').click()
      await flushPromises()

      expect(document.body.querySelector('.modal-dialog')).toBeNull()
    })
  })
})
