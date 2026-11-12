// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { fieldByLabel, openModals, dismissModal, renderWithPlugins } from '../test-utils'
import { dialogButton } from '../test-utils/dom'
import { menuLabels, openMenu } from '../test-utils/overflowMenu'
import { pushWs, resetWsMock } from '../test-utils/wsMock'
import {
  EMAIL_CHANNEL,
  MOCK_RULES,
  PUSH_CHANNEL,
  WEBHOOK_CHANNEL,
  delivery,
  scopeOptionsGet,
  stubPushBrowser,
  unstubPushBrowser,
} from '../test-utils/notificationFixtures'
import ChannelDetailView from './ChannelDetailView.vue'
import { useToast } from '../composables/useToast'
import { CHANNEL_HISTORY_FETCH } from '../utils/channelSettings'

vi.mock('../api/notifications', () => ({
  listChannels: vi.fn(),
  listRules: vi.fn(),
  listDeliveries: vi.fn(),
  getVapidPublicKey: vi.fn(),
  updateChannel: vi.fn(),
  deleteChannel: vi.fn(),
  testChannel: vi.fn(),
  createRule: vi.fn(),
  deleteRule: vi.fn(),
  subscribePush: vi.fn(),
  validateSmtp: vi.fn(),
  previewTemplate: vi.fn(),
}))

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

vi.mock('../utils/logger', () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
    info: vi.fn(),
  },
}))

import {
  createRule,
  deleteChannel,
  deleteRule,
  getVapidPublicKey,
  listChannels,
  listDeliveries,
  listRules,
  previewTemplate,
  subscribePush,
  testChannel,
  updateChannel,
  validateSmtp,
} from '../api/notifications'
import { apiClient } from '../api/client'
import type {
  NotificationChannelResponse as NotificationChannel,
  NotificationRuleResponse,
} from '../types/generated'

const mockListChannels = vi.mocked(listChannels)
const mockUpdateChannel = vi.mocked(updateChannel)
const mockValidateSmtp = vi.mocked(validateSmtp)

const PUSH_ON_LIST: NotificationChannel = PUSH_CHANNEL

function setupDefaultMocks(): void {
  mockListChannels.mockResolvedValue([WEBHOOK_CHANNEL, EMAIL_CHANNEL, PUSH_ON_LIST])
  vi.mocked(listRules).mockResolvedValue(MOCK_RULES)
  vi.mocked(listDeliveries).mockResolvedValue([])
  vi.mocked(getVapidPublicKey).mockResolvedValue({ key: '', configured: false })
  vi.mocked(previewTemplate).mockResolvedValue({ title: 'preview', body: 'preview' })
  vi.mocked(apiClient.get).mockImplementation(scopeOptionsGet as never)
}

type ViewWrapper = ReturnType<typeof renderWithPlugins>

/** A channel's page, with `section` open as `?section=` would open it. */
async function mountChannel(id: number, section?: string): Promise<ViewWrapper> {
  const wrapper = renderWithPlugins(ChannelDetailView, { props: { id: String(id) } })
  await wrapper.vm.$router.push({
    path: `/notifications/channels/${id}`,
    query: section ? { section } : {},
  })
  await flushPromises()
  return wrapper
}

function button(wrapper: ViewWrapper, label: string): ReturnType<ViewWrapper['find']> {
  const match = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!match) throw new Error(`no button labelled "${label}"`)
  return match
}

async function click(wrapper: ViewWrapper, label: string): Promise<void> {
  await button(wrapper, label).trigger('click')
  await flushPromises()
}

/** Opens or closes a section of the settings rail, as a click on its head does. */
async function toggleSection(wrapper: ViewWrapper, id: string): Promise<void> {
  await wrapper.find(`#acc-head-${id}`).trigger('click')
  await flushPromises()
}

/** Picks an action from the header's overflow menu. */
async function chooseMenuAction(wrapper: ViewWrapper, label: string): Promise<void> {
  await openMenu(wrapper)
  const item = wrapper.findAll('.overflow-menu-item').find((b) => b.text().trim() === label)
  if (!item) throw new Error(`no "${label}" in the channel menu`)
  await item.trigger('click')
  await flushPromises()
}

async function type(wrapper: ViewWrapper, label: string, value: string): Promise<void> {
  await fieldByLabel(wrapper, label).setValue(value)
  await flushPromises()
}

/** The toasts raised so far, as "type: message". */
function toastLines(): string[] {
  return useToast().toasts.value.map((t) => `${t.type}: ${t.message}`)
}

function infoGrid(wrapper: ViewWrapper | ReturnType<ViewWrapper['find']>): Record<string, string> {
  const values = wrapper.findAll('dd').map((d) => d.text())
  return Object.fromEntries(wrapper.findAll('dt').map((d, i) => [d.text(), values[i] ?? '']))
}

beforeEach(() => {
  vi.clearAllMocks()
  resetWsMock()
  setupDefaultMocks()
  useToast().toasts.value.splice(0)
})

describe('ChannelDetailView', () => {
  describe('loading', () => {
    it('heads the page with the channel, where it delivers and its state', async () => {
      const wrapper = await mountChannel(2)

      expect(wrapper.find('.detail-name').text()).toBe('Ops Email')
      expect(wrapper.find('.detail-subtitle').text()).toBe('Email · admin@example.com')
      expect(wrapper.findAll('.detail-title-row .badge').map((b) => b.text())).toEqual([
        'Email',
        'On',
      ])
      expect(wrapper.findAll('.detail-meta span').map((s) => s.text())).toEqual([
        'No events',
        'Scope: All',
        'Nothing sent yet',
      ])
      expect(wrapper.find('.crumb-current').text()).toBe('Ops Email')
      expect(wrapper.find('.crumb-link').attributes('href')).toBe('/notifications')
    })

    it('reads the history broadly, since the server cannot filter it by channel', async () => {
      await mountChannel(2)
      expect(vi.mocked(listDeliveries)).toHaveBeenCalledWith(CHANNEL_HISTORY_FETCH)
    })

    it('never shows a webhook’s token in the header', async () => {
      mockListChannels.mockResolvedValue([
        {
          ...WEBHOOK_CHANNEL,
          config: { url: 'https://user:hunter2@hooks.example.com/notify?token=abc' },
        } as NotificationChannel,
      ])
      const wrapper = await mountChannel(1)

      expect(wrapper.find('.detail-subtitle').text()).toBe('Webhook · hooks.example.com')
      expect(wrapper.find('.detail-header').text()).not.toContain('hunter2')
      expect(wrapper.find('.detail-meta').text()).toContain('1 of 14 events')
    })

    it('says so when the channel does not exist', async () => {
      const wrapper = await mountChannel(99)
      expect(wrapper.find('.error-banner').text()).toBe('Channel not found')
      expect(wrapper.find('.detail-header').exists()).toBe(false)
      expect(wrapper.find('.crumb-current').text()).toBe('...')
    })

    it('shows the load error in place of the page', async () => {
      mockListChannels.mockRejectedValue(new Error('server down'))
      const wrapper = await mountChannel(2)
      expect(wrapper.find('.error-banner').text()).toContain('server down')
    })

    it('loads the other channel when the route moves to it', async () => {
      const wrapper = await mountChannel(2)
      await wrapper.setProps({ id: '1' })
      await flushPromises()
      expect(wrapper.find('.detail-name').text()).toBe('Ops Webhook')
    })

    it('opens the section named in the URL', async () => {
      const wrapper = await mountChannel(2, 'link')
      expect(wrapper.find('#acc-head-link').attributes('aria-expanded')).toBe('true')
      expect(infoGrid(wrapper.find('#acc-body-link'))).toEqual({
        'Links to': 'Where the event happened',
      })
    })

    it('keeps the section last opened in the URL', async () => {
      const wrapper = await mountChannel(2)
      await toggleSection(wrapper, 'scope')
      expect(wrapper.vm.$router.currentRoute.value.query.section).toBe('scope')
    })
  })

  describe('delivery settings', () => {
    it('shows an email channel’s settings, but never its password', async () => {
      const wrapper = await mountChannel(2)
      expect(infoGrid(wrapper.find('#acc-body-settings'))).toEqual({
        Name: 'Ops Email',
        Type: 'Email',
        'SMTP server': 'smtp.example.com:587',
        Security: 'STARTTLS',
        'SMTP user': 'user',
        'SMTP password': 'Saved',
        From: 'noreply@example.com',
        To: 'admin@example.com',
      })
    })

    it('shows a webhook’s URL and its header names only', async () => {
      mockListChannels.mockResolvedValue([
        { ...WEBHOOK_CHANNEL, webhook_headers: [{ name: 'Authorization', has_value: true }] },
      ])
      const wrapper = await mountChannel(1)
      expect(infoGrid(wrapper.find('#acc-body-settings'))).toEqual({
        Name: 'Ops Webhook',
        Type: 'Webhook',
        URL: 'https://hooks.example.com/notify',
        Headers: 'Authorization',
      })
    })

    it('says where a web push channel delivers', async () => {
      const wrapper = await mountChannel(3)
      expect(infoGrid(wrapper.find('#acc-body-settings'))['Delivers to']).toBe(
        'Every browser subscribed to web push',
      )
    })

    it('prefills the form from the channel being edited', async () => {
      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      expect(wrapper.find<HTMLInputElement>('#channel-edit-name').element.value).toBe('Ops Email')
      expect((fieldByLabel(wrapper, 'SMTP host').element as HTMLInputElement).value).toBe(
        'smtp.example.com',
      )
    })

    // The wire type is a list; the field is one comma-separated string.
    it('joins the recipient list into the single address field', async () => {
      mockListChannels.mockResolvedValue([
        {
          ...EMAIL_CHANNEL,
          config: { ...EMAIL_CHANNEL.config, to_addresses: ['a@example.com', 'b@example.com'] },
        } as NotificationChannel,
      ])
      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      expect((fieldByLabel(wrapper, 'To addresses').element as HTMLInputElement).value).toBe(
        'a@example.com, b@example.com',
      )
    })

    it('shows the webhook URL instead for a webhook channel', async () => {
      const wrapper = await mountChannel(1)
      await click(wrapper, 'Edit')
      expect((fieldByLabel(wrapper, 'URL').element as HTMLInputElement).value).toBe(
        'https://hooks.example.com/notify',
      )
      expect(() => fieldByLabel(wrapper, 'SMTP host')).toThrow()
    })

    it('saves the edited channel and splits the recipients back into a list', async () => {
      mockValidateSmtp.mockResolvedValue(undefined)
      mockUpdateChannel.mockResolvedValue({ ...EMAIL_CHANNEL, name: 'Renamed' })

      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await wrapper.find('#channel-edit-name').setValue('Renamed')
      await type(wrapper, 'To addresses', 'a@example.com, b@example.com ,')
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).toHaveBeenCalledWith(
        2,
        expect.objectContaining({
          name: 'Renamed',
          config: expect.objectContaining({
            to_addresses: ['a@example.com', 'b@example.com'],
          }),
        }),
      )
      // The saved channel is what the page shows from then on.
      expect(wrapper.find('.detail-name').text()).toBe('Renamed')
      expect(wrapper.find('#channel-edit-name').exists()).toBe(false)
      expect(toastLines()).toEqual(['success: Channel saved'])
    })

    it('refuses to save without a name', async () => {
      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await wrapper.find('#channel-edit-name').setValue('   ')
      await click(wrapper, 'Save')

      expect(mockValidateSmtp).not.toHaveBeenCalled()
      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('.form-error').text()).toBe('Name is required')
    })

    // The server never returns the stored password, so the field starts blank
    // and a blank field means "keep it" - both for the SMTP check (which the
    // server runs against the saved channel's password) and for the save.
    it('starts the password blank and says one is saved', async () => {
      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      const password = wrapper.find<HTMLInputElement>('[data-testid="smtp-password"]').element
      expect(password.value).toBe('')
      expect(password.placeholder).toContain('leave blank to keep it')
      expect(wrapper.text()).toContain('A password is saved for this channel')
    })

    it('keeps the stored password when saved with the field blank', async () => {
      mockValidateSmtp.mockResolvedValue(undefined)
      mockUpdateChannel.mockResolvedValue(EMAIL_CHANNEL)

      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await click(wrapper, 'Save')

      expect(mockValidateSmtp).toHaveBeenCalledWith(
        expect.objectContaining({ smtp_password: '', channel_id: 2 }),
      )
      expect(mockUpdateChannel).toHaveBeenCalledWith(
        2,
        expect.objectContaining({ config: expect.objectContaining({ smtp_password: '' }) }),
      )
    })

    it('sends a newly typed password to replace the stored one', async () => {
      mockValidateSmtp.mockResolvedValue(undefined)
      mockUpdateChannel.mockResolvedValue(EMAIL_CHANNEL)

      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await type(wrapper, 'SMTP password', 'rotated-secret')
      await click(wrapper, 'Save')

      expect(mockValidateSmtp).toHaveBeenCalledWith(
        expect.objectContaining({ smtp_password: 'rotated-secret' }),
      )
      expect(mockUpdateChannel).toHaveBeenCalledWith(
        2,
        expect.objectContaining({
          config: expect.objectContaining({ smtp_password: 'rotated-secret' }),
        }),
      )
    })

    it('checks SMTP before saving an email channel', async () => {
      mockValidateSmtp.mockRejectedValue(new Error('535 auth failed'))

      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await click(wrapper, 'Save')

      expect(mockValidateSmtp).toHaveBeenCalledTimes(1)
      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('.form-error').text()).toBe('535 auth failed')
      expect(wrapper.find('#channel-edit-name').exists()).toBe(true)
    })

    // A webhook has no credentials to check, so it must not be sent through
    // the SMTP gate - doing so would make webhooks unsaveable.
    it('does not run the SMTP check for a webhook channel', async () => {
      mockUpdateChannel.mockResolvedValue(WEBHOOK_CHANNEL)

      const wrapper = await mountChannel(1)
      await click(wrapper, 'Edit')
      await click(wrapper, 'Save')

      expect(mockValidateSmtp).not.toHaveBeenCalled()
      expect(mockUpdateChannel).toHaveBeenCalledTimes(1)
    })

    it('saves only the name of a web push channel, which has no settings of its own', async () => {
      mockUpdateChannel.mockResolvedValue({ ...PUSH_CHANNEL, name: 'Laptop' })

      const wrapper = await mountChannel(3)
      await click(wrapper, 'Edit')
      expect(wrapper.find('[data-testid="smtp-password"]').exists()).toBe(false)
      expect(() => fieldByLabel(wrapper, 'URL')).toThrow()
      await wrapper.find('#channel-edit-name').setValue('Laptop')
      await click(wrapper, 'Save')

      expect(mockValidateSmtp).not.toHaveBeenCalled()
      expect(mockUpdateChannel).toHaveBeenCalledWith(3, { name: 'Laptop' })
    })

    it('reports a save failure without leaving the form', async () => {
      mockValidateSmtp.mockResolvedValue(undefined)
      mockUpdateChannel.mockRejectedValue(new Error('conflict'))

      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await click(wrapper, 'Save')

      expect(wrapper.find('.form-error').text()).toBe('conflict')
      expect(wrapper.find('#channel-edit-name').exists()).toBe(true)
      expect(toastLines()).toEqual([])
    })

    it('leaves the form without saving on Cancel', async () => {
      const wrapper = await mountChannel(2)
      await click(wrapper, 'Edit')
      await wrapper.find('#channel-edit-name').setValue('Discarded')
      await click(wrapper, 'Cancel')

      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('#channel-edit-name').exists()).toBe(false)
      expect(infoGrid(wrapper.find('#acc-body-settings')).Name).toBe('Ops Email')

      // Editing again starts from the saved channel, not the discarded draft.
      await click(wrapper, 'Edit')
      expect(wrapper.find<HTMLInputElement>('#channel-edit-name').element.value).toBe('Ops Email')
    })
  })

  describe('webhook headers', () => {
    const SAVED_HEADERS_WEBHOOK: NotificationChannel = {
      ...WEBHOOK_CHANNEL,
      webhook_headers: [{ name: 'Authorization', has_value: true }],
    }

    function headerInputs(wrapper: ViewWrapper, testId: string) {
      return wrapper.findAll<HTMLInputElement>(`[data-testid="${testId}"]`)
    }

    async function openWebhookEdit(): Promise<ViewWrapper> {
      mockListChannels.mockResolvedValue([SAVED_HEADERS_WEBHOOK, EMAIL_CHANNEL])
      mockUpdateChannel.mockResolvedValue(SAVED_HEADERS_WEBHOOK)
      const wrapper = await mountChannel(1)
      await click(wrapper, 'Edit')
      return wrapper
    }

    it('lists saved headers by name with their values left blank', async () => {
      const wrapper = await openWebhookEdit()
      expect(headerInputs(wrapper, 'webhook-header-name').map((i) => i.element.value)).toEqual([
        'Authorization',
      ])
      const [value] = headerInputs(wrapper, 'webhook-header-value')
      expect(value.element.value).toBe('')
      expect(value.element.type).toBe('password')
      expect(value.element.placeholder).toMatch(/leave blank to keep it/)
    })

    it('keeps a saved value when saved with the field blank', async () => {
      const wrapper = await openWebhookEdit()
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).toHaveBeenCalledWith(
        1,
        expect.objectContaining({
          config: expect.objectContaining({ headers: { Authorization: null } }),
        }),
      )
    })

    it('sends a newly typed value and a new header', async () => {
      const wrapper = await openWebhookEdit()
      await headerInputs(wrapper, 'webhook-header-value')[0].setValue('Bearer rotated')
      await wrapper.find('[data-testid="webhook-add-header"]').trigger('click')
      await headerInputs(wrapper, 'webhook-header-name')[1].setValue('X-Team')
      await headerInputs(wrapper, 'webhook-header-value')[1].setValue('ops')
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).toHaveBeenLastCalledWith(
        1,
        expect.objectContaining({
          config: expect.objectContaining({
            headers: { Authorization: 'Bearer rotated', 'X-Team': 'ops' },
          }),
        }),
      )
    })

    it('drops a removed header from the save', async () => {
      const wrapper = await openWebhookEdit()
      await wrapper.find('[aria-label="Remove header 1"]').trigger('click')
      expect(headerInputs(wrapper, 'webhook-header-name')).toHaveLength(0)
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).toHaveBeenLastCalledWith(
        1,
        expect.objectContaining({ config: expect.objectContaining({ headers: {} }) }),
      )
    })

    it('refuses to save two rows with the same header name', async () => {
      const wrapper = await openWebhookEdit()
      await wrapper.find('[data-testid="webhook-add-header"]').trigger('click')
      await headerInputs(wrapper, 'webhook-header-name')[1].setValue('authorization')
      await headerInputs(wrapper, 'webhook-header-value')[1].setValue('Bearer other')
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('.form-error').text()).toContain('Each header can only be given once')
      expect(wrapper.find('.form-error').text()).toContain('Authorization')
    })

    it('asks for saved values again once the URL points at another host', async () => {
      const wrapper = await openWebhookEdit()
      const reentry = () => wrapper.find('[data-testid="webhook-header-reentry"]')
      expect(reentry().exists()).toBe(false)

      await type(wrapper, 'URL', 'https://hooks.example.com/another-path')
      expect(reentry().exists()).toBe(false)

      await type(wrapper, 'URL', 'https://hooks.attacker.example/notify')
      expect(reentry().text()).toContain('Authorization')

      await headerInputs(wrapper, 'webhook-header-value')[0].setValue('Bearer entered-again')
      expect(reentry().exists()).toBe(false)
    })
  })

  describe('link', () => {
    async function openLinkEdit(channel: NotificationChannel = EMAIL_CHANNEL) {
      mockListChannels.mockResolvedValue([channel])
      const wrapper = await mountChannel(channel.id, 'link')
      const edit = wrapper.find('#acc-body-link').findAll('button')
      await edit.find((b) => b.text() === 'Edit')!.trigger('click')
      await flushPromises()
      return wrapper
    }

    function linkRadio(wrapper: ViewWrapper, target: string) {
      return wrapper.find<HTMLInputElement>(`#channel-link-${target}`)
    }

    it('says where the channel’s notifications lead', async () => {
      mockListChannels.mockResolvedValue([
        { ...EMAIL_CHANNEL, link: { target: 'custom', url: 'https://status.example.com' } },
      ])
      const wrapper = await mountChannel(2, 'link')
      expect(infoGrid(wrapper.find('#acc-body-link'))).toEqual({
        'Links to': 'https://status.example.com',
      })
    })

    it('starts the picker from the saved setting', async () => {
      const wrapper = await openLinkEdit({
        ...EMAIL_CHANNEL,
        link: { target: 'custom', url: 'https://status.example.com' },
      })
      expect(linkRadio(wrapper, 'custom').element.checked).toBe(true)
      expect(wrapper.find<HTMLInputElement>('#channel-link-url').element.value).toBe(
        'https://status.example.com',
      )
    })

    it('saves a new target with the channel’s name', async () => {
      mockUpdateChannel.mockResolvedValue({ ...EMAIL_CHANNEL, link: { target: 'dashboard' } })
      const wrapper = await openLinkEdit()

      await linkRadio(wrapper, 'dashboard').setValue(true)
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).toHaveBeenCalledWith(
        2,
        expect.objectContaining({ name: 'Ops Email', link: { target: 'dashboard' } }),
      )
      expect(infoGrid(wrapper.find('#acc-body-link'))).toEqual({ 'Links to': 'Dashboard' })
      expect(toastLines()).toEqual(['success: Link saved'])
    })

    it('asks for an address before saving your own', async () => {
      const wrapper = await openLinkEdit()
      await linkRadio(wrapper, 'custom').setValue(true)
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('#acc-body-link .form-error').text()).toBe('Needs an address to link to.')

      await wrapper.find('#channel-link-url').setValue('status.example.com')
      await click(wrapper, 'Save')
      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('#acc-body-link .form-error').text()).toBe(
        'Needs an address starting with http:// or https://.',
      )
    })

    it('saves your own address once it is one', async () => {
      const link = { target: 'custom' as const, url: 'https://status.example.com' }
      mockUpdateChannel.mockResolvedValue({ ...EMAIL_CHANNEL, link })
      const wrapper = await openLinkEdit()

      await linkRadio(wrapper, 'custom').setValue(true)
      await wrapper.find('#channel-link-url').setValue('https://status.example.com')
      await click(wrapper, 'Save')

      expect(mockUpdateChannel).toHaveBeenCalledWith(2, { name: 'Ops Email', link })
      expect(infoGrid(wrapper.find('#acc-body-link'))).toEqual({
        'Links to': 'https://status.example.com',
      })
    })

    it('keeps the form open with the error when saving fails', async () => {
      mockUpdateChannel.mockRejectedValue(new Error('not stored yet'))
      const wrapper = await openLinkEdit()

      await linkRadio(wrapper, 'none').setValue(true)
      await click(wrapper, 'Save')

      expect(wrapper.find('#acc-body-link .form-error').text()).toBe('not stored yet')
      expect(linkRadio(wrapper, 'none').element.checked).toBe(true)
      expect(toastLines()).toEqual([])
    })

    it('discards the choice on Cancel', async () => {
      const wrapper = await openLinkEdit()
      await linkRadio(wrapper, 'none').setValue(true)
      await click(wrapper, 'Cancel')

      expect(mockUpdateChannel).not.toHaveBeenCalled()
      expect(infoGrid(wrapper.find('#acc-body-link'))).toEqual({
        'Links to': 'Where the event happened',
      })
    })
  })

  describe('events', () => {
    function eventItem(wrapper: ViewWrapper, label: string) {
      const item = wrapper
        .findAll('#acc-body-events .event-item')
        .find((el) => el.find('.event-label').text() === label)
      if (!item) throw new Error(`no "${label}" event`)
      return item
    }

    function isOn(wrapper: ViewWrapper, label: string): boolean {
      return eventItem(wrapper, label).find('[role="switch"]').attributes('aria-checked') === 'true'
    }

    it('lists every event type as a switch, on for the channel’s rules only', async () => {
      const wrapper = await mountChannel(1, 'events')
      expect(wrapper.findAll('#acc-body-events .event-item')).toHaveLength(14)
      expect(isOn(wrapper, 'Backup failed')).toBe(true)
      expect(isOn(wrapper, 'Backup success')).toBe(false)
    })

    it('ignores another channel’s rules', async () => {
      const wrapper = await mountChannel(2, 'events')
      expect(isOn(wrapper, 'Backup failed')).toBe(false)
    })

    it('offers the deliberate wait for a host and a disabled schedule in plain words', async () => {
      const wrapper = await mountChannel(2, 'events')
      const labels = wrapper.findAll('#acc-body-events .event-label').map((l) => l.text())
      expect(labels).toContain('Backup waiting for host')
      expect(labels).toContain('Schedule auto disabled')
    })

    // A rule is a row, not a flag: enabling an event creates one and
    // disabling it deletes the existing row.
    it('creates a rule for an event that had none', async () => {
      vi.mocked(createRule).mockResolvedValue({
        id: 99,
        channel_id: 1,
        event_type: 'backup_success',
        repo_id: null,
        agent_id: null,
        enabled: true,
      })
      const wrapper = await mountChannel(1, 'events')

      await eventItem(wrapper, 'Backup success').find('[role="switch"]').trigger('click')
      await flushPromises()

      expect(vi.mocked(createRule)).toHaveBeenCalledWith({
        channel_id: 1,
        event_type: 'backup_success',
        enabled: true,
      })
      expect(isOn(wrapper, 'Backup success')).toBe(true)
      expect(wrapper.find('.detail-meta').text()).toContain('2 of 14 events')
    })

    it('deletes the rule when its event is switched off', async () => {
      vi.mocked(deleteRule).mockResolvedValue(undefined)
      const wrapper = await mountChannel(1, 'events')

      await eventItem(wrapper, 'Backup failed').find('[role="switch"]').trigger('click')
      await flushPromises()

      expect(vi.mocked(deleteRule)).toHaveBeenCalledWith(1)
      expect(isOn(wrapper, 'Backup failed')).toBe(false)
    })

    it('locks a switch while its change is saved', async () => {
      let resolve!: (rule: NotificationRuleResponse) => void
      vi.mocked(createRule).mockReturnValue(
        new Promise<NotificationRuleResponse>((r) => {
          resolve = r
        }),
      )
      const wrapper = await mountChannel(2, 'events')
      const toggle = (): ReturnType<ViewWrapper['find']> =>
        eventItem(wrapper, 'Backup success').find('[role="switch"]')

      await toggle().trigger('click')
      expect(toggle().attributes('disabled')).toBeDefined()
      // Only the one in flight: the others stay usable.
      expect(
        eventItem(wrapper, 'Backup failed').find('[role="switch"]').attributes('disabled'),
      ).toBeUndefined()

      resolve({
        id: 5,
        channel_id: 2,
        event_type: 'backup_success',
        repo_id: null,
        agent_id: null,
        enabled: true,
      })
      await flushPromises()
      expect(toggle().attributes('disabled')).toBeUndefined()
    })

    it('releases an event switch and says so when saving its rule fails', async () => {
      vi.mocked(createRule).mockRejectedValue(new Error('conflict'))
      const wrapper = await mountChannel(2, 'events')

      await eventItem(wrapper, 'Backup success').find('[role="switch"]').trigger('click')
      await flushPromises()

      expect(vi.mocked(createRule)).toHaveBeenCalledWith(
        expect.objectContaining({ channel_id: 2, enabled: true }),
      )
      expect(toastLines()).toEqual(['error: Could not change backup success: conflict'])
      expect(isOn(wrapper, 'Backup success')).toBe(false)
      expect(
        eventItem(wrapper, 'Backup success').find('[role="switch"]').attributes('disabled'),
      ).toBeUndefined()
    })
  })

  describe('scope', () => {
    function scopeBoxes(wrapper: ViewWrapper) {
      return wrapper.findAll<HTMLInputElement>('#acc-body-scope .scope-item input[type="checkbox"]')
    }

    it('offers the loaded repositories, agents and schedules by name', async () => {
      const wrapper = await mountChannel(1, 'scope')
      const text = wrapper.find('#acc-body-scope').text()
      expect(text).toContain('server-daily')
      expect(text).toContain('web-01')
      expect(text).toContain('Nightly web')
      expect(scopeBoxes(wrapper)).toHaveLength(3)
    })

    it('narrows the scope list as you search', async () => {
      const wrapper = await mountChannel(1, 'scope')
      const before = wrapper.findAll('#acc-body-scope .scope-item').length

      await wrapper.find('#acc-body-scope .scope-search').setValue('server-daily')
      await flushPromises()

      const after = wrapper.findAll('#acc-body-scope .scope-item')
      expect(after.length).toBeLessThan(before)
      expect(after.map((i) => i.text()).join(' ')).toContain('server-daily')
    })

    it('narrows an existing channel from the scope list', async () => {
      mockUpdateChannel.mockResolvedValue({ ...WEBHOOK_CHANNEL, scope: { repo_ids: [7] } })
      const wrapper = await mountChannel(1, 'scope')

      await scopeBoxes(wrapper)[0].setValue(true)
      await flushPromises()

      expect(mockUpdateChannel).toHaveBeenCalledWith(1, { scope: { repo_ids: [7] } })
      expect(wrapper.find('.detail-meta').text()).toContain('Scope: 1 repo')
    })

    // Scope is additive: ticking another kind must not drop what was ticked before.
    it('narrows an existing channel by agent and schedule too', async () => {
      mockUpdateChannel.mockImplementation(
        async (id: number, body: { scope?: unknown }) =>
          ({ ...WEBHOOK_CHANNEL, id, scope: body.scope ?? {} }) as NotificationChannel,
      )
      const wrapper = await mountChannel(1, 'scope')

      for (const box of scopeBoxes(wrapper)) {
        await box.setValue(true)
        await flushPromises()
      }

      expect(mockUpdateChannel.mock.calls.map((c) => c[1])).toEqual([
        { scope: { repo_ids: [7] } },
        { scope: { repo_ids: [7], agent_ids: [3] } },
        { scope: { repo_ids: [7], agent_ids: [3], schedule_ids: [5] } },
      ])
    })

    it('widens the channel again when an item is unticked', async () => {
      mockListChannels.mockResolvedValue([{ ...WEBHOOK_CHANNEL, scope: { repo_ids: [7] } }])
      mockUpdateChannel.mockResolvedValue({ ...WEBHOOK_CHANNEL, scope: { repo_ids: [] } })
      const wrapper = await mountChannel(1, 'scope')

      expect(scopeBoxes(wrapper)[0].element.checked).toBe(true)
      await scopeBoxes(wrapper)[0].setValue(false)
      await flushPromises()

      expect(mockUpdateChannel).toHaveBeenCalledWith(1, { scope: { repo_ids: [] } })
    })

    it('keeps the scope unchanged and says so when saving it fails', async () => {
      mockUpdateChannel.mockRejectedValue(new Error('offline'))
      const wrapper = await mountChannel(1, 'scope')

      await scopeBoxes(wrapper)[0].setValue(true)
      await flushPromises()

      expect(mockUpdateChannel).toHaveBeenCalledWith(1, { scope: { repo_ids: [7] } })
      expect(toastLines()).toEqual(['error: Could not change the scope: offline'])
      expect(wrapper.find('.detail-meta').text()).toContain('Scope: All')
    })

    it('still shows the channel when the scope options fail to load', async () => {
      vi.mocked(apiClient.get).mockRejectedValue(new Error('scope down'))
      const { logger } = await import('../utils/logger')
      const wrapper = await mountChannel(1, 'scope')

      expect(vi.mocked(logger.error).mock.calls.map((c) => c[0])).toContain(
        'loadScopeOptions failed',
      )
      expect(wrapper.find('.detail-name').text()).toBe('Ops Webhook')
      expect(scopeBoxes(wrapper)).toHaveLength(0)
    })
  })

  describe('content', () => {
    it('reflects a saved content template back onto the channel', async () => {
      mockUpdateChannel.mockResolvedValue({
        ...EMAIL_CHANNEL,
        config: { ...EMAIL_CHANNEL.config, title_template: 'Custom title' },
      } as NotificationChannel)
      const wrapper = await mountChannel(2, 'content')

      await wrapper.find('#content-title-2').setValue('Custom title')
      await flushPromises()

      const save = (): ReturnType<ViewWrapper['find']> => button(wrapper, 'Save content')
      expect(save().attributes('disabled')).toBeUndefined()
      await save().trigger('click')
      await flushPromises()

      expect(mockUpdateChannel).toHaveBeenCalledWith(
        2,
        expect.objectContaining({
          config: expect.objectContaining({ title_template: 'Custom title' }),
        }),
      )
      // The Save button only re-disables once the channel prop it reads its
      // "saved" value from has been updated with the response - i.e. once the
      // page's channel was replaced, not just the editor's own local state.
      expect(save().attributes('disabled')).toBeDefined()
    })
  })

  describe('delivery history', () => {
    it('lists only this channel’s deliveries, newest first, with no channel filter', async () => {
      vi.mocked(listDeliveries).mockResolvedValue([
        delivery(1, 2, 'sent', 30),
        delivery(2, 1, 'failed', 1, 'another channel'),
        delivery(3, 2, 'failed', 2, 'SMTP auth failed'),
      ])
      const wrapper = await mountChannel(2, 'history')

      const rows = wrapper.findAll('[data-testid="delivery-history"] .log-row')
      expect(rows.map((r) => r.find('.log-row-facts').text())).toEqual(['SMTP auth failed', 'Sent'])
      expect(wrapper.find('#acc-body-history select').exists()).toBe(false)
      expect(wrapper.findAll('.detail-meta span')[2].text()).toBe(
        'Failed 2h ago · SMTP auth failed',
      )
    })

    it('has the shared empty state before anything was sent', async () => {
      const wrapper = await mountChannel(2, 'history')
      expect(wrapper.find('#acc-body-history .empty-title').text()).toBe('No deliveries yet')
    })

    it('adds a live delivery for this channel and ignores the others', async () => {
      const wrapper = await mountChannel(2, 'history')

      pushWs('NotificationDelivery', delivery(10, 1, 'sent', 0))
      await flushPromises()
      expect(wrapper.find('[data-testid="delivery-history"]').exists()).toBe(false)

      pushWs('NotificationDelivery', delivery(11, 2, 'failed', 0, 'live failure'))
      await flushPromises()
      const rows = wrapper.findAll('[data-testid="delivery-history"] .log-row')
      expect(rows).toHaveLength(1)
      expect(rows[0].find('.log-row-facts').text()).toBe('live failure')
    })
  })

  describe('header actions', () => {
    it('offers a test, pausing and deleting in the menu', async () => {
      const wrapper = await mountChannel(1)
      await openMenu(wrapper)
      expect(menuLabels(wrapper)).toEqual(['Send test', 'Pause channel', 'Delete channel'])
    })

    it('reports a successful test send in a toast', async () => {
      vi.mocked(testChannel).mockResolvedValue(undefined)
      const wrapper = await mountChannel(1)

      await chooseMenuAction(wrapper, 'Send test')

      expect(vi.mocked(testChannel)).toHaveBeenCalledWith(1)
      expect(toastLines()).toEqual(['success: Test sent to Ops Webhook'])
    })

    it('reports a failed test send in a toast', async () => {
      vi.mocked(testChannel).mockRejectedValue(new Error('connection refused'))
      const wrapper = await mountChannel(2)

      await chooseMenuAction(wrapper, 'Send test')

      expect(toastLines()).toEqual(['error: Test to Ops Email failed: connection refused'])
      // The menu entry is usable again once the attempt has settled.
      await openMenu(wrapper)
      const item = wrapper.findAll('.overflow-menu-item').find((b) => b.text() === 'Send test')
      expect(item?.attributes('disabled')).toBeUndefined()
    })

    it('pauses a channel from the menu', async () => {
      mockUpdateChannel.mockResolvedValue({ ...WEBHOOK_CHANNEL, enabled: false })
      const wrapper = await mountChannel(1)

      await chooseMenuAction(wrapper, 'Pause channel')

      expect(mockUpdateChannel).toHaveBeenCalledWith(1, { enabled: false })
      expect(wrapper.findAll('.detail-title-row .badge')[1].text()).toBe('Paused')
      expect(toastLines()).toEqual(['success: Paused Ops Webhook'])
      // The menu now offers the way back.
      await openMenu(wrapper)
      expect(menuLabels(wrapper)).toContain('Resume channel')
    })

    it('resumes a paused channel from the menu', async () => {
      mockListChannels.mockResolvedValue([{ ...WEBHOOK_CHANNEL, enabled: false }])
      mockUpdateChannel.mockResolvedValue(WEBHOOK_CHANNEL)
      const wrapper = await mountChannel(1)

      await chooseMenuAction(wrapper, 'Resume channel')

      expect(mockUpdateChannel).toHaveBeenCalledWith(1, { enabled: true })
      expect(wrapper.findAll('.detail-title-row .badge')[1].text()).toBe('On')
      expect(toastLines()).toEqual(['success: Resumed Ops Webhook'])
    })

    it('leaves a channel on when pausing it fails', async () => {
      mockUpdateChannel.mockRejectedValue(new Error('offline'))
      const wrapper = await mountChannel(1)

      await chooseMenuAction(wrapper, 'Pause channel')

      expect(toastLines()).toEqual(['error: offline'])
      expect(wrapper.findAll('.detail-title-row .badge')[1].text()).toBe('On')
    })

    it('names the channel it is about to delete', async () => {
      const wrapper = await mountChannel(1)
      await chooseMenuAction(wrapper, 'Delete channel')
      expect(openModals(wrapper)).toHaveLength(1)
      expect(wrapper.find('.confirm-text').text()).toContain('Delete channel Ops Webhook?')
    })

    it('deletes the channel on confirmation and returns to the list', async () => {
      vi.mocked(deleteChannel).mockResolvedValue(undefined)
      const wrapper = await mountChannel(1)

      await chooseMenuAction(wrapper, 'Delete channel')
      dialogButton('Delete').click()
      await flushPromises()

      expect(vi.mocked(deleteChannel)).toHaveBeenCalledWith(1)
      expect(toastLines()).toEqual(['success: Deleted Ops Webhook'])
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications')
      expect(openModals(wrapper)).toHaveLength(0)
    })

    it('keeps the channel when the delete is cancelled', async () => {
      const wrapper = await mountChannel(1)
      await chooseMenuAction(wrapper, 'Delete channel')
      dialogButton('Cancel').click()
      await flushPromises()

      expect(vi.mocked(deleteChannel)).not.toHaveBeenCalled()
      expect(openModals(wrapper)).toHaveLength(0)
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications/channels/1')
    })

    // Escape and the backdrop close the dialog through BaseModal, which is
    // wired separately from its own Cancel button. Nothing may be written.
    it('closes the delete dialog when it is dismissed', async () => {
      const wrapper = await mountChannel(1)
      await chooseMenuAction(wrapper, 'Delete channel')

      await dismissModal(wrapper)

      expect(openModals(wrapper)).toHaveLength(0)
      expect(vi.mocked(deleteChannel)).not.toHaveBeenCalled()
    })

    it('keeps the delete dialog open with the error when deleting fails', async () => {
      vi.mocked(deleteChannel).mockRejectedValue(new Error('in use'))
      const wrapper = await mountChannel(1)

      await chooseMenuAction(wrapper, 'Delete channel')
      dialogButton('Delete').click()
      await flushPromises()

      expect(wrapper.find('.modal-dialog .form-error').text()).toBe('in use')
      expect(openModals(wrapper)).toHaveLength(1)
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications/channels/1')
    })
  })

  describe('web push test', () => {
    beforeEach(() => {
      mockListChannels.mockResolvedValue([PUSH_CHANNEL])
      vi.mocked(getVapidPublicKey).mockResolvedValue({ key: 'AQID-_8', configured: true })
    })

    afterEach(() => {
      unstubPushBrowser()
    })

    it('asks again after a dismissed prompt, then tests through a fresh subscription', async () => {
      vi.mocked(testChannel).mockResolvedValue(undefined)
      const browser = stubPushBrowser('default', ['default', 'granted'])
      const wrapper = await mountChannel(3)

      await chooseMenuAction(wrapper, 'Send test')

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
      const stubbed = browser ? stubPushBrowser(browser.permission, browser.answers) : null
      const wrapper = await mountChannel(3)

      await chooseMenuAction(wrapper, 'Send test')

      expect(vi.mocked(testChannel)).not.toHaveBeenCalled()
      if (stubbed) expect(stubbed.pushManager.subscribe).not.toHaveBeenCalled()
      expect(toastLines()).toHaveLength(1)
      expect(toastLines()[0]).toMatch(/^error: Test to My Browser failed: /)
    })

    it('does not subscribe when the server has no VAPID keys', async () => {
      const browser = stubPushBrowser('granted', [])
      vi.mocked(getVapidPublicKey).mockResolvedValue({ key: '', configured: false })
      const wrapper = await mountChannel(3)

      await chooseMenuAction(wrapper, 'Send test')

      expect(browser.pushManager.getSubscription).not.toHaveBeenCalled()
      expect(browser.pushManager.subscribe).not.toHaveBeenCalled()
      expect(vi.mocked(testChannel)).not.toHaveBeenCalled()
      expect(toastLines()).toEqual([
        'error: Test to My Browser failed: VAPID keys not configured on the server',
      ])
    })
  })
})
