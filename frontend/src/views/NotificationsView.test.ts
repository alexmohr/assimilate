// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises, type DOMWrapper } from '@vue/test-utils'
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import NotificationsView from './NotificationsView.vue'
import { useToast } from '../composables/useToast'

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

import type {
  EmailConfig,
  NotificationChannelResponse as NotificationChannel,
  NotificationDeliveryResponse as NotificationDelivery,
  NotificationRuleResponse as NotificationRule,
  WebhookConfig,
} from '../types/generated'

const WEBHOOK_CHANNEL: NotificationChannel = {
  id: 1,
  name: 'Ops Webhook',
  channel_type: 'webhook',
  config: { url: 'https://hooks.example.com/notify' } as WebhookConfig,
  has_password: false,
  webhook_headers: [],
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
    from_address: 'noreply@example.com',
    to_addresses: ['admin@example.com'],
    security: 'starttls',
  } as EmailConfig,
  has_password: true,
  webhook_headers: [],
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

type ViewWrapper = ReturnType<typeof renderWithPlugins>

/** The channel rows, in the order the list shows them. */
function channelRows(wrapper: ViewWrapper): DOMWrapper<Element>[] {
  return wrapper.findAll('.channel-row')
}

function channelRow(wrapper: ViewWrapper, name: string): DOMWrapper<Element> {
  const row = channelRows(wrapper).find((r) => r.attributes('data-channel') === name)
  if (!row) throw new Error(`no channel row for "${name}"`)
  return row
}

/** Opens a row's overflow menu and picks one of its actions. */
async function chooseMenuAction(row: DOMWrapper<Element>, label: string): Promise<void> {
  await row.find('.overflow-toggle').trigger('click')
  const item = row.findAll('[role="menuitem"]').find((b) => b.text() === label)
  if (!item) throw new Error(`no "${label}" in the channel menu`)
  await item.trigger('click')
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

  it('puts every channel action in the row menu rather than on the row', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const rows = channelRows(wrapper)
    expect(rows).toHaveLength(2)
    for (const row of rows) {
      // The menu toggle is the only button a row carries.
      expect(row.findAll('button').map((b) => b.classes())).toEqual([
        expect.arrayContaining(['overflow-toggle']),
      ])
      expect(row.findComponent({ name: 'ToggleSwitch' }).exists()).toBe(false)
    }

    await rows[0].find('.overflow-toggle').trigger('click')
    expect(rows[0].findAll('[role="menuitem"]').map((b) => b.text())).toEqual([
      'Edit',
      'Edit content',
      'Edit events',
      'Edit scope',
      'Send test',
      'Pause',
      'Delete',
    ])
  })

  it('opening the row menu does not open the edit dialog', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    await channelRows(wrapper)[0].find('.overflow-toggle').trigger('click')
    await flushPromises()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('opens the edit dialog from the keyboard too', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    await channelRow(wrapper, 'Ops Email').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(fieldByLabel('Name').value).toBe('Ops Email')
  })

  it('summarises each channel on its row without its secrets', async () => {
    setupDefaultMocks()
    mockListChannels.mockResolvedValue([
      {
        ...WEBHOOK_CHANNEL,
        config: { url: 'https://user:hunter2@hooks.example.com/notify?token=abc' },
      } as NotificationChannel,
      { ...EMAIL_CHANNEL, enabled: false, scope: { repo_ids: [1, 2] } } as NotificationChannel,
    ])
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()

    const hook = channelRow(wrapper, 'Ops Webhook')
    expect(hook.find('.list-row-sub').text()).toBe('Webhook \u00b7 hooks.example.com')
    expect(hook.text()).not.toContain('hunter2')
    expect(hook.text()).not.toContain('token')
    expect(hook.find('.channel-events').text()).toBe('1 of 14 events')
    expect(hook.find('.channel-scope').text()).toBe('All')
    expect(hook.find('.badge').text()).toBe('On')
    expect(hook.find('.channel-delivery').text()).toBe('Nothing sent yet')

    const email = channelRow(wrapper, 'Ops Email')
    expect(email.find('.list-row-sub').text()).toBe('Email \u00b7 admin@example.com')
    expect(email.find('.channel-events').text()).toBe('No events')
    expect(email.find('.channel-scope').text()).toBe('2 repos')
    expect(email.find('.badge').text()).toBe('Paused')
    expect(email.find('.channel-summary').text()).toBe('No events \u00b7 2 repos \u00b7 Paused')
  })

  it('counts the channels on their tab', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    const tab = wrapper.findAll('[role="tab"]').find((t) => t.text().includes('Channels'))
    expect(tab?.text()).toContain('2')
  })

  it('renders New button for adding a channel', async () => {
    setupDefaultMocks()
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    expect(wrapper.text()).toContain('New')
  })

  it('reflects a saved content template back onto its channel card', async () => {
    setupDefaultMocks()
    const { updateChannel } = await import('../api/notifications')
    vi.mocked(updateChannel).mockResolvedValue({
      ...EMAIL_CHANNEL,
      config: { ...EMAIL_CHANNEL.config, title_template: 'Custom title' },
    } as never)

    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()

    await chooseMenuAction(channelRow(wrapper, 'Ops Email'), 'Edit content')

    const titleInput = document.body.querySelector<HTMLInputElement>('#content-title-2')!
    titleInput.value = 'Custom title'
    titleInput.dispatchEvent(new Event('input'))
    await flushPromises()

    const saveBtn = dialogButton('Save content')
    expect(saveBtn.disabled).toBe(false)
    saveBtn.click()
    await flushPromises()

    // The Save button only re-disables once the channel prop it reads its "saved" value
    // from has been updated with the response from updateChannel - i.e. once the parent's
    // channels list was updated, not just the child's own local state.
    expect(dialogButton('Save content').disabled).toBe(true)

    dialogButton('Done').click()
    await flushPromises()
    expect(openModals(wrapper)).toHaveLength(0)
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
    expect(document.body.textContent).toContain('New channel')
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

    // Clicking the webhook channel's row opens its editor.
    await channelRow(wrapper, 'Ops Webhook').trigger('click')
    await flushPromises()

    // Edit dialog should show webhook URL field - exercises editChannelWebhookCfg and isWebhookConfig
    expect(document.body.textContent).toContain('URL')
  })

  describe('add channel wizard', () => {
    /** Fills step 1 with a valid email channel and advances to step 2. */
    async function fillStepOne(): Promise<void> {
      await setByLabel('Name', 'Ops Alerts')
      await setByLabel('SMTP host', 'smtp.example.com')
      await setByLabel('From address', 'noreply@example.com')
      await setByLabel('To addresses', 'admin@example.com')
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

      expect(document.body.querySelectorAll('.event-item')).toHaveLength(14)
    })

    it('offers the deliberate wait for a host as its own event', async () => {
      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()

      const items = [...document.body.querySelectorAll('.event-item')]
      expect(items.some((el) => el.textContent?.includes('Backup waiting for host'))).toBe(true)
    })

    it('labels the schedule-auto-disabled event type in plain words', async () => {
      await openWizard()
      await fillStepOne()
      dialogButton('Next').click()
      await flushPromises()

      const items = [...document.body.querySelectorAll('.event-item')]
      expect(items.some((el) => el.textContent?.includes('Schedule auto disabled'))).toBe(true)
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
          l.textContent?.includes('SMTP host'),
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
    /** Opens the edit dialog for a channel by its row position, through the row menu. */
    async function openEdit(index: number) {
      setupDefaultMocks()
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      await chooseMenuAction(channelRows(wrapper)[index], 'Edit')
      return wrapper
    }

    it('prefills the dialog from the channel being edited', async () => {
      await openEdit(1)
      expect((fieldByLabel('Name') as HTMLInputElement).value).toBe('Ops Email')
      expect((fieldByLabel('SMTP host') as HTMLInputElement).value).toBe('smtp.example.com')
    })

    // The wire type is a list; the field is one comma-separated string.
    it('joins the recipient list into the single address field', async () => {
      await openEdit(1)
      expect((fieldByLabel('To addresses') as HTMLInputElement).value).toBe('admin@example.com')
    })

    it('shows the webhook URL instead for a webhook channel', async () => {
      await openEdit(0)
      expect((fieldByLabel('URL') as HTMLInputElement).value).toBe(
        'https://hooks.example.com/notify',
      )
      expect(() => fieldByLabel('SMTP host')).toThrow()
    })

    it('saves the edited channel and splits the recipients back into a list', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(updateChannel).mockResolvedValue({ ...EMAIL_CHANNEL, name: 'Renamed' } as never)

      await openEdit(1)
      await setByLabel('Name', 'Renamed')
      await setByLabel('To addresses', 'a@example.com, b@example.com ,')
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

    // The server never returns the stored password, so the field starts blank
    // and a blank field means "keep it" - both for the SMTP check (which the
    // server runs against the saved channel's password) and for the save.
    it('starts the password blank and says one is saved', async () => {
      await openEdit(1)
      const password = fieldByLabel('SMTP password') as HTMLInputElement
      expect(password.value).toBe('')
      expect(password.placeholder).toContain('leave blank to keep it')
      expect(document.body.textContent).toContain('A password is saved for this channel')
    })

    it('keeps the stored password when saved with the field blank', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(updateChannel).mockResolvedValue(EMAIL_CHANNEL as never)

      await openEdit(1)
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(validateSmtp)).toHaveBeenCalledWith(
        expect.objectContaining({ smtp_password: '', channel_id: 2 }),
      )
      expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(
        2,
        expect.objectContaining({ config: expect.objectContaining({ smtp_password: '' }) }),
      )
    })

    it('sends a newly typed password to replace the stored one', async () => {
      const { updateChannel, validateSmtp } = await import('../api/notifications')
      vi.mocked(validateSmtp).mockResolvedValue({} as never)
      vi.mocked(updateChannel).mockResolvedValue(EMAIL_CHANNEL as never)

      await openEdit(1)
      await setByLabel('SMTP password', 'rotated-secret')
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(validateSmtp)).toHaveBeenCalledWith(
        expect.objectContaining({ smtp_password: 'rotated-secret' }),
      )
      expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(
        2,
        expect.objectContaining({
          config: expect.objectContaining({ smtp_password: 'rotated-secret' }),
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

  describe('webhook headers', () => {
    const SAVED_HEADERS_WEBHOOK: NotificationChannel = {
      ...WEBHOOK_CHANNEL,
      webhook_headers: [{ name: 'Authorization', has_value: true }],
    }

    function headerInputs(testId: string): HTMLInputElement[] {
      return [...document.body.querySelectorAll<HTMLInputElement>(`[data-testid="${testId}"]`)]
    }

    async function type(control: HTMLInputElement, value: string): Promise<void> {
      control.value = value
      control.dispatchEvent(new Event('input'))
      await flushPromises()
    }

    async function openWebhookEdit(): Promise<void> {
      setupDefaultMocks()
      mockListChannels.mockResolvedValue([SAVED_HEADERS_WEBHOOK, EMAIL_CHANNEL])
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      await channelRows(wrapper)[0].trigger('click')
      await flushPromises()
    }

    it('lists saved headers by name with their values left blank', async () => {
      await openWebhookEdit()
      expect(headerInputs('webhook-header-name').map((i) => i.value)).toEqual(['Authorization'])
      const [value] = headerInputs('webhook-header-value')
      expect(value.value).toBe('')
      expect(value.type).toBe('password')
      expect(value.placeholder).toMatch(/leave blank to keep it/)
    })

    it('keeps a saved value when saved with the field blank', async () => {
      const { updateChannel } = await import('../api/notifications')
      vi.mocked(updateChannel).mockResolvedValue(SAVED_HEADERS_WEBHOOK as never)

      await openWebhookEdit()
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(
        1,
        expect.objectContaining({
          config: expect.objectContaining({ headers: { Authorization: null } }),
        }),
      )
    })

    it('sends a newly typed value, a new header, and drops a removed one', async () => {
      const { updateChannel } = await import('../api/notifications')
      vi.mocked(updateChannel).mockResolvedValue(SAVED_HEADERS_WEBHOOK as never)

      await openWebhookEdit()
      await type(headerInputs('webhook-header-value')[0], 'Bearer rotated')
      ;(document.body.querySelector('[data-testid="webhook-add-header"]') as HTMLElement).click()
      await flushPromises()
      await type(headerInputs('webhook-header-name')[1], 'X-Team')
      await type(headerInputs('webhook-header-value')[1], 'ops')
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).toHaveBeenLastCalledWith(
        1,
        expect.objectContaining({
          config: expect.objectContaining({
            headers: { Authorization: 'Bearer rotated', 'X-Team': 'ops' },
          }),
        }),
      )
    })

    it('refuses to save two rows with the same header name', async () => {
      const { updateChannel } = await import('../api/notifications')

      await openWebhookEdit()
      ;(document.body.querySelector('[data-testid="webhook-add-header"]') as HTMLElement).click()
      await flushPromises()
      await type(headerInputs('webhook-header-name')[1], 'authorization')
      await type(headerInputs('webhook-header-value')[1], 'Bearer other')
      dialogButton('Save').click()
      await flushPromises()

      expect(vi.mocked(updateChannel)).not.toHaveBeenCalled()
      expect(document.body.querySelector('.form-error')?.textContent).toContain('Authorization')
    })

    it('removes a header row', async () => {
      await openWebhookEdit()
      ;(document.body.querySelector('[aria-label="Remove header 1"]') as HTMLElement).click()
      await flushPromises()
      expect(headerInputs('webhook-header-name')).toHaveLength(0)
    })

    it('asks for saved values again once the URL points at another host', async () => {
      await openWebhookEdit()
      const reentry = (): Element | null =>
        document.body.querySelector('[data-testid="webhook-header-reentry"]')
      expect(reentry()).toBeNull()

      await setByLabel('URL', 'https://hooks.example.com/another-path')
      expect(reentry()).toBeNull()

      await setByLabel('URL', 'https://hooks.attacker.example/notify')
      expect(reentry()?.textContent).toContain('Authorization')

      await type(headerInputs('webhook-header-value')[0], 'Bearer entered-again')
      expect(reentry()).toBeNull()
    })

    it('refuses to create a webhook channel with the same header twice', async () => {
      const { createChannel } = await import('../api/notifications')

      setupDefaultMocks()
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('New'))!
        .trigger('click')
      await flushPromises()
      const typeSelect = document.body.querySelector('select')!
      typeSelect.value = 'webhook'
      typeSelect.dispatchEvent(new Event('change', { bubbles: true }))
      await flushPromises()

      await setByLabel('Name', 'Ops Hook')
      await setByLabel('URL', 'https://hooks.example.com/notify')
      const addHeader = document.body.querySelector(
        '[data-testid="webhook-add-header"]',
      ) as HTMLElement
      addHeader.click()
      await flushPromises()
      addHeader.click()
      await flushPromises()
      await type(headerInputs('webhook-header-name')[0], 'Authorization')
      await type(headerInputs('webhook-header-name')[1], 'authorization')
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Create').click()
      await flushPromises()

      expect(vi.mocked(createChannel)).not.toHaveBeenCalled()
      expect(document.body.querySelector('.form-error')?.textContent).toContain(
        'Each header can only be given once',
      )
    })

    it('creates a webhook channel with its headers', async () => {
      const { createChannel } = await import('../api/notifications')
      vi.mocked(createChannel).mockResolvedValue({ ...WEBHOOK_CHANNEL, id: 7 } as never)

      setupDefaultMocks()
      const wrapper = renderWithPlugins(NotificationsView)
      await flushPromises()
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('New'))!
        .trigger('click')
      await flushPromises()
      const typeSelect = document.body.querySelector('select')!
      typeSelect.value = 'webhook'
      typeSelect.dispatchEvent(new Event('change', { bubbles: true }))
      await flushPromises()

      await setByLabel('Name', 'Ops Hook')
      await setByLabel('URL', 'https://hooks.example.com/notify')
      ;(document.body.querySelector('[data-testid="webhook-add-header"]') as HTMLElement).click()
      await flushPromises()
      await type(headerInputs('webhook-header-name')[0], 'Authorization')
      await type(headerInputs('webhook-header-value')[0], 'Bearer new-token')
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Next').click()
      await flushPromises()
      dialogButton('Create').click()
      await flushPromises()

      expect(vi.mocked(createChannel)).toHaveBeenCalledWith(
        expect.objectContaining({
          channel_type: 'webhook',
          config: expect.objectContaining({
            url: 'https://hooks.example.com/notify',
            headers: { Authorization: 'Bearer new-token' },
          }),
        }),
      )
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

    /** Picks an action from the first channel's row menu. */
    async function clickByTitle(wrapper: Awaited<ReturnType<typeof render>>, title: string) {
      await chooseMenuAction(channelRows(wrapper)[0], title)
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
      await set('SMTP host', 'smtp.example.com')
      await set('From address', 'noreply@example.com')
      await set('To addresses', 'admin@example.com')

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
      await clickByTitle(wrapper, 'Delete')
      expect(document.body.textContent).toContain('Ops Webhook')
    })

    it('deletes the channel on confirmation and drops its rules', async () => {
      const { deleteChannel } = await import('../api/notifications')
      vi.mocked(deleteChannel).mockResolvedValue(undefined as never)

      const wrapper = await render()
      await clickByTitle(wrapper, 'Delete')
      dialogButton('Delete').click()
      await flushPromises()

      expect(vi.mocked(deleteChannel)).toHaveBeenCalledWith(1)
      expect(wrapper.text()).not.toContain('Ops Webhook')
    })

    it('keeps the channel when the delete is cancelled', async () => {
      const { deleteChannel } = await import('../api/notifications')
      const wrapper = await render()
      await clickByTitle(wrapper, 'Delete')
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

    /** Picks an action from the first channel's row menu. */
    async function clickByTitle(wrapper: ReturnType<typeof renderWithPlugins>, title: string) {
      await chooseMenuAction(channelRows(wrapper)[0], title)
    }

    async function fillStepOne(): Promise<void> {
      await setByLabel('Name', 'Scoped')
      await setByLabel('SMTP host', 'smtp.example.com')
      await setByLabel('From address', 'noreply@example.com')
      await setByLabel('To addresses', 'admin@example.com')
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
        el.textContent?.includes('failed'),
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
      await channelRow(wrapper, 'Ops Email').trigger('click')
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

    // Escape and the backdrop close a dialog through BaseModal, which every
    // dialog in this view wires separately from its own footer button. Nothing
    // may be written on the way out.
    it.each([
      [
        'new channel',
        async (w: ReturnType<typeof renderWithPlugins>): Promise<void> => openWizard(w),
      ],
      [
        'edit channel',
        async (w: ReturnType<typeof renderWithPlugins>): Promise<void> => {
          await channelRows(w)[0].trigger('click')
          await flushPromises()
        },
      ],
      [
        'delete channel',
        async (w: ReturnType<typeof renderWithPlugins>): Promise<void> => {
          await clickByTitle(w, 'Delete')
        },
      ],
      [
        'events',
        async (w: ReturnType<typeof renderWithPlugins>): Promise<void> =>
          clickByTitle(w, 'Edit events'),
      ],
      [
        'scope',
        async (w: ReturnType<typeof renderWithPlugins>): Promise<void> =>
          clickByTitle(w, 'Edit scope'),
      ],
      [
        'content',
        async (w: ReturnType<typeof renderWithPlugins>): Promise<void> =>
          clickByTitle(w, 'Edit content'),
      ],
    ])('closes the %s dialog when it is dismissed', async (_name, open) => {
      const { createChannel, updateChannel, deleteChannel } = await import('../api/notifications')
      const wrapper = await render()
      await open(wrapper)
      expect(openModals(wrapper)).toHaveLength(1)

      await dismissModal(wrapper)

      expect(openModals(wrapper)).toHaveLength(0)
      expect(vi.mocked(createChannel)).not.toHaveBeenCalled()
      expect(vi.mocked(updateChannel)).not.toHaveBeenCalled()
      expect(vi.mocked(deleteChannel)).not.toHaveBeenCalled()
    })
  })
})

describe('NotificationsView channel actions', () => {
  const PUSH_CHANNEL: NotificationChannel = {
    ...WEBHOOK_CHANNEL,
    id: 3,
    name: 'My Browser',
    channel_type: 'web_push',
    config: {} as WebhookConfig,
  }

  beforeEach(() => {
    vi.clearAllMocks()
    setupDefaultMocks()
    useToast().toasts.value.splice(0)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    Reflect.deleteProperty(navigator, 'serviceWorker')
  })

  async function mountView() {
    const wrapper = renderWithPlugins(NotificationsView)
    await flushPromises()
    return wrapper
  }

  function card(wrapper: Awaited<ReturnType<typeof mountView>>, name: string) {
    return channelRow(wrapper, name)
  }

  async function pressTest(wrapper: Awaited<ReturnType<typeof mountView>>, name: string) {
    await chooseMenuAction(channelRow(wrapper, name), 'Send test')
  }

  /** The toasts raised so far, as "type: message". */
  function toastLines(): string[] {
    return useToast().toasts.value.map((t) => `${t.type}: ${t.message}`)
  }

  async function loggedMessages(): Promise<unknown[]> {
    const { logger } = await import('../utils/logger')
    return vi.mocked(logger.error).mock.calls.map((c) => c[0])
  }

  /**
   * jsdom has no service worker or Push API, so a browser that supports web
   * push is assembled here: a ready registration whose push manager hands out
   * subscriptions, and a Notification permission that answers in turn.
   */
  function stubPushBrowser(permission: NotificationPermission, answers: NotificationPermission[]) {
    const existing = {
      toJSON: () => ({ endpoint: 'https://push.example.com/old' }),
      unsubscribe: vi.fn().mockResolvedValue(true),
    }
    const fresh = { toJSON: () => ({ endpoint: 'https://push.example.com/new' }) }
    const pushManager = {
      getSubscription: vi.fn().mockResolvedValue(existing),
      subscribe: vi.fn().mockResolvedValue(fresh),
    }
    Object.defineProperty(navigator, 'serviceWorker', {
      configurable: true,
      value: { ready: Promise.resolve({ pushManager }) },
    })
    vi.stubGlobal('PushManager', class {})
    const requestPermission = vi.fn()
    for (const answer of answers) requestPermission.mockResolvedValueOnce(answer)
    vi.stubGlobal('Notification', { permission, requestPermission })
    return { existing, pushManager, requestPermission }
  }

  it('reports a successful test send in a toast', async () => {
    const { testChannel } = await import('../api/notifications')
    vi.mocked(testChannel).mockResolvedValue(undefined as never)
    const wrapper = await mountView()

    await pressTest(wrapper, 'Ops Webhook')

    expect(vi.mocked(testChannel)).toHaveBeenCalledWith(1)
    expect(toastLines()).toEqual(['success: Test sent to Ops Webhook'])
  })

  it('reports a failed test send in a toast', async () => {
    const { testChannel } = await import('../api/notifications')
    vi.mocked(testChannel).mockRejectedValue(new Error('connection refused'))
    const wrapper = await mountView()

    await pressTest(wrapper, 'Ops Email')

    expect(toastLines()).toEqual(['error: Test to Ops Email failed: Unknown error'])
    // The menu entry is usable again once the attempt has settled.
    const row = card(wrapper, 'Ops Email')
    await row.find('.overflow-toggle').trigger('click')
    const item = row.findAll('[role="menuitem"]').find((b) => b.text() === 'Send test')
    expect(item?.attributes('disabled')).toBeUndefined()
  })

  it('pauses a channel from its row menu', async () => {
    const { updateChannel } = await import('../api/notifications')
    vi.mocked(updateChannel).mockResolvedValue({ ...WEBHOOK_CHANNEL, enabled: false })
    const wrapper = await mountView()

    await chooseMenuAction(card(wrapper, 'Ops Webhook'), 'Pause')

    expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(1, { enabled: false })
    expect(card(wrapper, 'Ops Webhook').find('.badge').text()).toBe('Paused')
    expect(card(wrapper, 'Ops Email').find('.badge').text()).toBe('On')
    // The menu now offers the way back, and opening it never opened the editor.
    await card(wrapper, 'Ops Webhook').find('.overflow-toggle').trigger('click')
    expect(card(wrapper, 'Ops Webhook').text()).toContain('Resume')
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('resumes a paused channel from its row menu', async () => {
    const { updateChannel } = await import('../api/notifications')
    mockListChannels.mockResolvedValue([{ ...WEBHOOK_CHANNEL, enabled: false }, EMAIL_CHANNEL])
    vi.mocked(updateChannel).mockResolvedValue(WEBHOOK_CHANNEL)
    const wrapper = await mountView()

    await chooseMenuAction(card(wrapper, 'Ops Webhook'), 'Resume')

    expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(1, { enabled: true })
    expect(card(wrapper, 'Ops Webhook').find('.badge').text()).toBe('On')
  })

  it('leaves a channel on when pausing it fails', async () => {
    const { updateChannel } = await import('../api/notifications')
    vi.mocked(updateChannel).mockRejectedValue(new Error('offline'))
    const wrapper = await mountView()

    await chooseMenuAction(card(wrapper, 'Ops Webhook'), 'Pause')

    expect(await loggedMessages()).toContain('toggleChannel failed')
    expect(card(wrapper, 'Ops Webhook').find('.badge').text()).toBe('On')
  })

  describe('delivery health', () => {
    const at = (hoursAgo: number): string =>
      new Date(Date.now() - hoursAgo * 3_600_000).toISOString()

    function delivery(
      id: number,
      channelId: number,
      status: 'sent' | 'failed',
      hoursAgo: number,
      error: string | null = null,
    ): NotificationDelivery {
      return {
        id,
        channel_id: channelId,
        event_type: 'backup_failed',
        status,
        error_message: error,
        payload: {},
        attempted_at: at(hoursAgo),
      } as unknown as NotificationDelivery
    }

    const HISTORY = [
      delivery(1, 2, 'failed', 2, 'SMTP auth failed: 535 bad credentials'),
      delivery(2, 2, 'failed', 5, 'older'),
      delivery(3, 1, 'sent', 24),
      delivery(4, 2, 'sent', 30),
      delivery(5, 1, 'sent', 48),
      delivery(6, 1, 'sent', 50),
      delivery(7, 1, 'sent', 60),
    ]

    it('has no failure notice while every channel delivers', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY.filter((d) => d.status === 'sent'))
      const wrapper = await mountView()
      expect(wrapper.find('.notice-bar').exists()).toBe(false)
    })

    it('calls out a failing channel with its count and last error', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      const notices = wrapper.findAll('.notice-bar--danger')
      expect(notices).toHaveLength(1)
      expect(notices[0].find('.notice-bar-title').text()).toBe(
        'Ops Email failed 2 of its last 3 deliveries',
      )
      expect(notices[0].text()).toContain('Last error: SMTP auth failed: 535 bad credentials')
    })

    it('lists the failing channel first, with its last delivery in tone', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      expect(channelRows(wrapper).map((r) => r.attributes('data-channel'))).toEqual([
        'Ops Email',
        'Ops Webhook',
      ])
      const email = card(wrapper, 'Ops Email').find('.channel-delivery')
      expect(email.classes()).toContain('channel-delivery--danger')
      expect(email.text()).toBe('Failed 2h ago \u00b7 SMTP auth failed: 535 bad credentials')
      const hook = card(wrapper, 'Ops Webhook').find('.channel-delivery')
      expect(hook.classes()).not.toContain('channel-delivery--danger')
      expect(hook.text()).toBe('Sent 1d ago')
    })

    it('opens the history narrowed to the failing channel', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      const view = wrapper.findAll('button').find((b) => b.text() === 'View history')!
      await view.trigger('click')
      await flushPromises()

      expect(wrapper.find('[role="tab"][aria-selected="true"]').text()).toContain('History')
      expect(wrapper.find<HTMLSelectElement>('.toolbar select').element.value).toBe('2')
      expect(wrapper.findAll('.delivery-row')).toHaveLength(3)
    })

    it('shows the five newest deliveries under the channels', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      const rows = wrapper.findAll('.recent-group .list-row')
      expect(rows).toHaveLength(5)
      expect(rows[0].find('.recent-channel').text()).toBe('Ops Email')
      expect(rows[0].find('.recent-detail').text()).toBe('SMTP auth failed: 535 bad credentials')
      expect(rows[0].find('.recent-when').text()).toBe('2h ago')
      expect(rows[2].find('.recent-detail').text()).toBe('Delivered')
    })

    it('opens the full, unfiltered history from the recent deliveries', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      const all = wrapper.findAll('button').find((b) => b.text() === 'All history')!
      await all.trigger('click')
      await flushPromises()

      expect(wrapper.findAll('.delivery-row')).toHaveLength(HISTORY.length)
      expect(wrapper.find<HTMLSelectElement>('.toolbar select').element.value).toBe('')
    })

    it('keeps the channel picked in the history filter across tab switches', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      const all = wrapper.findAll('button').find((b) => b.text() === 'All history')!
      await all.trigger('click')
      await flushPromises()
      await wrapper.find<HTMLSelectElement>('.toolbar select').setValue('2')
      await flushPromises()
      expect(wrapper.findAll('.delivery-row')).toHaveLength(3)

      // The pick lives on the page, not in the tab, so leaving the tab and
      // coming back finds the history still narrowed to that channel.
      const tab = (label: string) =>
        wrapper.findAll('[role="tab"]').find((t) => t.text().includes(label))!
      await tab('Channels').trigger('click')
      await flushPromises()
      await tab('History').trigger('click')
      await flushPromises()
      expect(wrapper.find<HTMLSelectElement>('.toolbar select').element.value).toBe('2')
      expect(wrapper.findAll('.delivery-row')).toHaveLength(3)
    })

    it('has no recent deliveries panel before anything was sent', async () => {
      const wrapper = await mountView()
      expect(wrapper.find('.recent-group').exists()).toBe(false)
      expect(card(wrapper, 'Ops Webhook').find('.channel-delivery').text()).toBe('Nothing sent yet')
    })
  })

  it('keeps the delete dialog open with the error when deleting fails', async () => {
    const { deleteChannel } = await import('../api/notifications')
    vi.mocked(deleteChannel).mockRejectedValue(new Error('in use'))
    const wrapper = await mountView()

    await chooseMenuAction(card(wrapper, 'Ops Webhook'), 'Delete')
    dialogButton('Delete').click()
    await flushPromises()

    expect(document.body.querySelector('.form-error')?.textContent?.trim()).toBe('Unknown error')
    expect(openModals(wrapper)).toHaveLength(1)
    expect(wrapper.text()).toContain('Ops Webhook')
  })

  it('still lists the channels when history and scope options fail to load', async () => {
    mockListDeliveries.mockRejectedValue(new Error('history down'))
    mockApiGet.mockRejectedValue(new Error('scope down'))
    const wrapper = await mountView()

    const logged = await loggedMessages()
    expect(logged).toContain('loadDeliveries failed')
    expect(logged).toContain('loadScopeOptions failed')
    expect(channelRows(wrapper)).toHaveLength(2)
  })

  it('releases an event switch when saving its rule fails', async () => {
    const { createRule } = await import('../api/notifications')
    vi.mocked(createRule).mockRejectedValue(new Error('conflict'))
    const wrapper = await mountView()
    await chooseMenuAction(card(wrapper, 'Ops Email'), 'Edit events')

    const first = document.body.querySelector('.event-item input, .event-item button')
    ;(first as HTMLElement).click()
    await flushPromises()

    expect(vi.mocked(createRule)).toHaveBeenCalledWith(
      expect.objectContaining({ channel_id: 2, enabled: true }),
    )
    expect(await loggedMessages()).toContain('toggleRule failed')
    expect((first as HTMLInputElement).disabled).toBe(false)
  })

  it('keeps the scope unchanged when saving it fails', async () => {
    const { updateChannel } = await import('../api/notifications')
    vi.mocked(updateChannel).mockRejectedValue(new Error('offline'))
    mockApiGet.mockImplementation((url: string) =>
      Promise.resolve({ data: url === '/repos' ? [{ id: 7, name: 'server-daily' }] : [] }),
    )
    const wrapper = await mountView()
    await chooseMenuAction(card(wrapper, 'Ops Webhook'), 'Edit scope')

    const box = document.body.querySelector<HTMLInputElement>('.scope-item input[type="checkbox"]')
    box!.click()
    await flushPromises()

    expect(vi.mocked(updateChannel)).toHaveBeenCalledWith(1, { scope: { repo_ids: [7] } })
    expect(await loggedMessages()).toContain('toggleScopeItem failed')
    expect(card(wrapper, 'Ops Webhook').text()).not.toContain('server-daily')
  })

  it('drops an event that was picked and then unpicked in the wizard', async () => {
    const { createChannel, createRule, validateSmtp } = await import('../api/notifications')
    vi.mocked(validateSmtp).mockResolvedValue({} as never)
    vi.mocked(createChannel).mockResolvedValue({ ...EMAIL_CHANNEL, id: 42, name: 'Picky' })
    vi.mocked(createRule).mockImplementation(
      async (req: unknown) => ({ id: 5, ...(req as object) }) as never,
    )
    const wrapper = await mountView()
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('New'))!
      .trigger('click')
    await flushPromises()
    await setByLabel('Name', 'Picky')
    await setByLabel('SMTP host', 'smtp.example.com')
    await setByLabel('From address', 'noreply@example.com')
    await setByLabel('To addresses', 'admin@example.com')
    dialogButton('Next').click()
    await flushPromises()

    const switchAt = (i: number): HTMLElement =>
      document.body.querySelectorAll('.event-item')[i].querySelector('input, button') as HTMLElement
    switchAt(0).click()
    await flushPromises()
    switchAt(1).click()
    await flushPromises()
    switchAt(0).click()
    await flushPromises()

    dialogButton('Next').click()
    await flushPromises()
    dialogButton('Create').click()
    await flushPromises()

    // Only the second event (backup_warning) survives; the first was toggled off again.
    expect(vi.mocked(createRule).mock.calls.map((c) => c[0].event_type)).toEqual(['backup_warning'])
  })

  describe('web push', () => {
    beforeEach(() => {
      mockListChannels.mockResolvedValue([PUSH_CHANNEL])
      mockGetVapidPublicKey.mockResolvedValue({ key: 'AQID-_8', configured: true })
    })

    it('re-registers the browser subscription it finds on load', async () => {
      const { subscribePush } = await import('../api/notifications')
      stubPushBrowser('granted', [])

      await mountView()

      expect(vi.mocked(subscribePush)).toHaveBeenCalledWith({
        endpoint: 'https://push.example.com/old',
      })
    })

    it('asks again after a dismissed prompt, then tests through a fresh subscription', async () => {
      const { subscribePush, testChannel } = await import('../api/notifications')
      vi.mocked(testChannel).mockResolvedValue(undefined as never)
      const browser = stubPushBrowser('default', ['default', 'granted'])
      const wrapper = await mountView()
      vi.mocked(subscribePush).mockClear()

      await pressTest(wrapper, 'My Browser')

      expect(browser.requestPermission).toHaveBeenCalledTimes(2)
      expect(browser.existing.unsubscribe).toHaveBeenCalledTimes(1)
      // The URL-safe base64 VAPID key is decoded to raw bytes for the browser.
      expect(browser.pushManager.subscribe).toHaveBeenCalledWith({
        userVisibleOnly: true,
        applicationServerKey: new Uint8Array([1, 2, 3, 0xfb, 0xff]),
      })
      expect(vi.mocked(subscribePush)).toHaveBeenCalledWith({
        endpoint: 'https://push.example.com/new',
      })
      expect(vi.mocked(testChannel)).toHaveBeenCalledWith(3)
      expect(toastLines()).toEqual(['success: Test sent to My Browser'])
    })

    it.each([
      ['the browser has no Push API', null],
      ['notifications are blocked', { permission: 'denied' as const, answers: [] }],
      ['the prompt is refused', { permission: 'default' as const, answers: ['denied' as const] }],
    ])('does not send a test when %s', async (_case, browser) => {
      const { testChannel } = await import('../api/notifications')
      const stubbed = browser ? stubPushBrowser(browser.permission, browser.answers) : null
      const wrapper = await mountView()

      await pressTest(wrapper, 'My Browser')

      expect(vi.mocked(testChannel)).not.toHaveBeenCalled()
      if (stubbed) expect(stubbed.pushManager.subscribe).not.toHaveBeenCalled()
      expect(toastLines()).toHaveLength(1)
      expect(toastLines()[0]).toMatch(/^error: Test to My Browser failed: /)
    })

    it('does not subscribe when the server has no VAPID keys', async () => {
      const { testChannel } = await import('../api/notifications')
      const browser = stubPushBrowser('granted', [])
      mockGetVapidPublicKey.mockResolvedValue({ key: '', configured: false })
      const wrapper = await mountView()

      await pressTest(wrapper, 'My Browser')

      expect(browser.pushManager.getSubscription).not.toHaveBeenCalled()
      expect(browser.pushManager.subscribe).not.toHaveBeenCalled()
      expect(vi.mocked(testChannel)).not.toHaveBeenCalled()
      expect(toastLines()).toHaveLength(1)
      expect(toastLines()[0]).toMatch(/^error: Test to My Browser failed: /)
    })
  })
})
