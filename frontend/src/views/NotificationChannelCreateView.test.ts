// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { fieldByLabel, renderWithPlugins } from '../test-utils'
import {
  EMAIL_CHANNEL,
  PUSH_CHANNEL,
  WEBHOOK_CHANNEL,
  scopeOptionsGet,
  stubPushBrowser,
  unstubPushBrowser,
} from '../test-utils/notificationFixtures'
import NotificationChannelCreateView from './NotificationChannelCreateView.vue'

vi.mock('../api/notifications', () => ({
  createChannel: vi.fn(),
  createRule: vi.fn(),
  getVapidPublicKey: vi.fn(),
  subscribePush: vi.fn(),
  validateSmtp: vi.fn(),
}))

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../utils/logger', () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
    info: vi.fn(),
  },
}))

import {
  createChannel,
  createRule,
  getVapidPublicKey,
  subscribePush,
  validateSmtp,
} from '../api/notifications'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'
import type { CreateRuleRequest, NotificationRuleResponse } from '../types/generated'

const mockCreateChannel = vi.mocked(createChannel)
const mockCreateRule = vi.mocked(createRule)
const mockValidateSmtp = vi.mocked(validateSmtp)

type ViewWrapper = ReturnType<typeof renderWithPlugins>

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(getVapidPublicKey).mockResolvedValue({ key: '', configured: false })
  vi.mocked(apiClient.get).mockImplementation(scopeOptionsGet as never)
  mockValidateSmtp.mockResolvedValue(undefined)
  mockCreateChannel.mockResolvedValue({ ...EMAIL_CHANNEL, id: 42 })
  mockCreateRule.mockImplementation(
    async (req: CreateRuleRequest) =>
      ({ id: 1, repo_id: null, agent_id: null, ...req }) as NotificationRuleResponse,
  )
})

async function mountWizard(): Promise<ViewWrapper> {
  const wrapper = renderWithPlugins(NotificationChannelCreateView)
  await wrapper.vm.$router.push('/notifications/new')
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

async function type(wrapper: ViewWrapper, label: string, value: string): Promise<void> {
  await fieldByLabel(wrapper, label).setValue(value)
  await flushPromises()
}

/** Fills the delivery step with a valid email channel. */
async function fillEmail(wrapper: ViewWrapper, name = 'Ops Alerts'): Promise<void> {
  await type(wrapper, 'Name', name)
  await type(wrapper, 'SMTP host', 'smtp.example.com')
  await type(wrapper, 'From address', 'noreply@example.com')
  await type(wrapper, 'To addresses', 'admin@example.com')
}

async function chooseType(wrapper: ViewWrapper, value: string): Promise<void> {
  await wrapper.find('#channel-type').setValue(value)
  await flushPromises()
}

/** Continues `times` steps forward. */
async function next(wrapper: ViewWrapper, times = 1): Promise<void> {
  for (let i = 0; i < times; i++) await click(wrapper, 'Continue')
}

/** From the delivery step to the review, where Create sits. */
async function toReview(wrapper: ViewWrapper): Promise<void> {
  await next(wrapper, 4)
}

function caption(wrapper: ViewWrapper): string {
  return wrapper.find('.wizard-caption').text()
}

function footStatus(wrapper: ViewWrapper): string {
  return wrapper.find('.wizard-status').text()
}

function eventSwitch(wrapper: ViewWrapper, label: string): ReturnType<ViewWrapper['find']> {
  const item = wrapper.findAll('.event-item').find((el) => el.find('.event-label').text() === label)
  if (!item) throw new Error(`no "${label}" event`)
  return item.find('[role="switch"]')
}

/** A review block's text, by its group label. */
function reviewBlock(wrapper: ViewWrapper, label: string): ReturnType<ViewWrapper['find']> {
  const block = wrapper
    .findAll('.review-block')
    .find((b) => b.find('.group-label').text() === label)
  if (!block) throw new Error(`no "${label}" review block`)
  return block
}

describe('NotificationChannelCreateView', () => {
  describe('steps', () => {
    it('opens on the delivery step of five', async () => {
      const wrapper = await mountWizard()
      expect(caption(wrapper)).toBe('Step 1 of 5 · Delivery')
      expect(wrapper.findAll('.wizard-step').map((s) => s.find('span span').text())).toEqual([
        'Delivery',
        'Events',
        'Scope',
        'Link',
        'Review',
      ])
      expect(wrapper.find<HTMLSelectElement>('#channel-type').element.value).toBe('email')
    })

    // The delivery step is the only one with required fields, so it is the
    // only one that blocks Continue; the others may all be left alone.
    it('will not continue past delivery until the channel is configured', async () => {
      const wrapper = await mountWizard()
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
      expect(footStatus(wrapper)).toBe(
        'Needs a name and an SMTP server and a sender and a recipient to continue.',
      )

      await type(wrapper, 'Name', 'Ops Alerts')
      expect(footStatus(wrapper)).toBe(
        'Needs an SMTP server and a sender and a recipient to continue.',
      )

      await fillEmail(wrapper)
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
      expect(footStatus(wrapper)).toBe('Step 1 of 5.')
    })

    it('does not count a name of blanks', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper, '   ')
      expect(footStatus(wrapper)).toBe('Needs a name to continue.')
    })

    it('walks forward and back through the steps', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)

      await next(wrapper)
      expect(caption(wrapper)).toBe('Step 2 of 5 · Events')
      expect(wrapper.text()).toContain('Which events this channel is told about.')

      await next(wrapper)
      expect(caption(wrapper)).toBe('Step 3 of 5 · Scope')

      await click(wrapper, 'Back')
      expect(caption(wrapper)).toBe('Step 2 of 5 · Events')
    })

    it('offers Cancel on every step, and Back after the first', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      expect(button(wrapper, 'Cancel').exists()).toBe(true)
      expect(button(wrapper, 'Back').attributes('disabled')).toBeDefined()

      await next(wrapper)

      expect(button(wrapper, 'Cancel').exists()).toBe(true)
      expect(button(wrapper, 'Back').attributes('disabled')).toBeUndefined()
    })

    it('returns to the list without creating anything on Cancel', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await next(wrapper, 2)
      await click(wrapper, 'Cancel')

      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications')
      expect(mockCreateChannel).not.toHaveBeenCalled()
    })

    it('starts from a clean form each time it is opened', async () => {
      const first = await mountWizard()
      await type(first, 'Name', 'discarded')
      await click(first, 'Cancel')
      first.unmount()

      const wrapper = await mountWizard()
      expect((fieldByLabel(wrapper, 'Name').element as HTMLInputElement).value).toBe('')
      expect(caption(wrapper)).toBe('Step 1 of 5 · Delivery')
    })

    it('jumps to a step from the rail, and the Create button names what is still missing', async () => {
      const wrapper = await mountWizard()
      await type(wrapper, 'Name', 'Ops Alerts')

      const review = wrapper.findAll('.wizard-step').find((s) => s.text().includes('Review'))!
      await review.trigger('click')
      await flushPromises()

      expect(caption(wrapper)).toBe('Step 5 of 5 · Review')
      expect(button(wrapper, 'Create channel').attributes('disabled')).toBeDefined()
      expect(footStatus(wrapper)).toBe(
        'Still needs an SMTP server (Delivery), a sender (Delivery), a recipient (Delivery).',
      )
    })
  })

  describe('delivery', () => {
    it('switches to the webhook fields', async () => {
      const wrapper = await mountWizard()
      await chooseType(wrapper, 'webhook')

      expect(() => fieldByLabel(wrapper, 'URL')).not.toThrow()
      expect(() => fieldByLabel(wrapper, 'SMTP host')).toThrow()
      expect(footStatus(wrapper)).toBe('Needs a name and a webhook URL to continue.')
    })

    it('marks the required fields', async () => {
      const wrapper = await mountWizard()
      const required = wrapper
        .findAll('.field')
        .filter((f) => f.find('.required').exists())
        .map((f) => f.find('.field-label').text().replace('*', '').trim())
      expect(required).toEqual(['Type', 'Name', 'SMTP host', 'From address', 'To addresses'])
    })

    it('refuses web push until the server has its keys', async () => {
      const wrapper = await mountWizard()
      await chooseType(wrapper, 'web_push')
      await type(wrapper, 'Name', 'Laptop')

      expect(wrapper.find('.wizard-note--warning').text()).toContain(
        'Web push needs its keys configured on the server',
      )
      expect(footStatus(wrapper)).toBe('Needs push keys configured on the server to continue.')
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
    })

    it('accepts web push once the server has its keys', async () => {
      vi.mocked(getVapidPublicKey).mockResolvedValue({ key: 'AQID-_8', configured: true })
      const wrapper = await mountWizard()
      await chooseType(wrapper, 'web_push')
      await type(wrapper, 'Name', 'Laptop')

      expect(wrapper.find('.wizard-note--warning').exists()).toBe(false)
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
    })

    it('treats a push key it cannot read as missing', async () => {
      vi.mocked(getVapidPublicKey).mockRejectedValue(new Error('down'))
      const wrapper = await mountWizard()
      await chooseType(wrapper, 'web_push')
      await type(wrapper, 'Name', 'Laptop')

      expect(vi.mocked(logger.error).mock.calls.map((c) => c[0])).toContain(
        'loading the push key failed',
      )
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
    })
  })

  describe('events', () => {
    it('lists every event type as a switch, all off', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await next(wrapper)

      const switches = wrapper.findAll('.event-item [role="switch"]')
      expect(switches).toHaveLength(14)
      expect(switches.every((s) => s.attributes('aria-checked') === 'false')).toBe(true)
    })

    it('offers the deliberate wait for a host as its own event', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await next(wrapper)
      expect(wrapper.findAll('.event-label').map((l) => l.text())).toContain(
        'Backup waiting for host',
      )
    })

    it('labels the schedule-auto-disabled event type in plain words', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await next(wrapper)
      expect(wrapper.findAll('.event-label').map((l) => l.text())).toContain(
        'Schedule auto disabled',
      )
    })

    it('creates the channel and one rule per selected event, then opens it', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await next(wrapper)
      await eventSwitch(wrapper, 'Backup success').trigger('click')
      await eventSwitch(wrapper, 'Backup failed').trigger('click')
      expect(eventSwitch(wrapper, 'Backup success').attributes('aria-checked')).toBe('true')

      await next(wrapper, 3)
      expect(reviewBlock(wrapper, 'Events').find('p').text()).toBe('Backup success, Backup failed')
      await click(wrapper, 'Create channel')

      expect(mockCreateChannel).toHaveBeenCalledTimes(1)
      // One rule per event picked, each bound to the new channel.
      expect(mockCreateRule.mock.calls.map((c) => c[0])).toEqual([
        { channel_id: 42, event_type: 'backup_success', enabled: true },
        { channel_id: 42, event_type: 'backup_failed', enabled: true },
      ])
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications/channels/42')
    })

    it('creates no rules when no event was picked', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await toReview(wrapper)
      expect(reviewBlock(wrapper, 'Events').find('p').text()).toBe('None yet')
      await click(wrapper, 'Create channel')

      expect(mockCreateChannel).toHaveBeenCalledTimes(1)
      expect(mockCreateRule).not.toHaveBeenCalled()
    })

    it('drops an event that was picked and then unpicked', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper, 'Picky')
      await next(wrapper)

      await eventSwitch(wrapper, 'Backup success').trigger('click')
      await eventSwitch(wrapper, 'Backup warning').trigger('click')
      await eventSwitch(wrapper, 'Backup success').trigger('click')

      await next(wrapper, 3)
      await click(wrapper, 'Create channel')

      // Only the second event survives; the first was toggled off again.
      expect(mockCreateRule.mock.calls.map((c) => c[0].event_type)).toEqual(['backup_warning'])
    })
  })

  describe('creating an email channel', () => {
    it('sends the channel as typed, with the recipients split into a list', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await type(wrapper, 'To addresses', 'a@example.com, b@example.com ,')
      await toReview(wrapper)
      await click(wrapper, 'Create channel')

      expect(mockCreateChannel).toHaveBeenCalledWith({
        name: 'Ops Alerts',
        enabled: true,
        scope: {},
        link: { target: 'automatic' },
        channel_type: 'email',
        config: expect.objectContaining({
          smtp_host: 'smtp.example.com',
          from_address: 'noreply@example.com',
          to_addresses: ['a@example.com', 'b@example.com'],
        }),
      })
    })

    // A channel whose SMTP credentials cannot log in would silently swallow
    // every notification, so the wizard verifies before it stores anything.
    it('refuses to create an email channel whose SMTP login fails', async () => {
      mockValidateSmtp.mockRejectedValue(new Error('535 auth failed'))

      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await toReview(wrapper)
      await click(wrapper, 'Create channel')

      expect(mockValidateSmtp).toHaveBeenCalledTimes(1)
      expect(mockCreateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('.form-error').text()).toBe('535 auth failed')
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications/new')
    })

    /**
     * The SMTP fields live on the first step, so by the time Create is
     * pressed on the last one the fields component is unmounted. Gating the
     * save on a ref to it meant the check silently never ran and every email
     * channel was rejected. The check has to survive its fields leaving the
     * screen.
     */
    it('validates SMTP from the review step, where the fields are no longer rendered', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await toReview(wrapper)

      expect(() => fieldByLabel(wrapper, 'SMTP host')).toThrow()
      await click(wrapper, 'Create channel')

      expect(mockValidateSmtp).toHaveBeenCalledWith(
        expect.objectContaining({ smtp_host: 'smtp.example.com', smtp_password: '' }),
      )
      // A new channel has no stored password to fall back on.
      expect(mockValidateSmtp.mock.calls[0][0]).not.toHaveProperty('channel_id')
      expect(mockCreateChannel).toHaveBeenCalledTimes(1)
    })

    it('reports a create failure without leaving the wizard', async () => {
      mockCreateChannel.mockRejectedValue(new Error('duplicate name'))

      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await toReview(wrapper)
      await click(wrapper, 'Create channel')

      expect(caption(wrapper)).toBe('Step 5 of 5 · Review')
      expect(wrapper.find('.form-error').text()).toBe('duplicate name')
      expect(button(wrapper, 'Create channel').attributes('disabled')).toBeUndefined()
    })

    it('creates a disabled channel when the enable switch is turned off', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)

      await wrapper.find('[role="switch"][aria-label="Enable immediately"]').trigger('click')
      await toReview(wrapper)
      expect(reviewBlock(wrapper, 'Delivery').text()).toContain('Paused')
      await click(wrapper, 'Create channel')

      expect(mockCreateChannel).toHaveBeenCalledWith(expect.objectContaining({ enabled: false }))
    })
  })

  describe('creating a webhook channel', () => {
    function headerInputs(wrapper: ViewWrapper, testId: string) {
      return wrapper.findAll<HTMLInputElement>(`[data-testid="${testId}"]`)
    }

    async function fillWebhook(wrapper: ViewWrapper): Promise<void> {
      await chooseType(wrapper, 'webhook')
      await type(wrapper, 'Name', 'Ops Hook')
      await type(wrapper, 'URL', 'https://hooks.example.com/notify')
    }

    it('creates a webhook channel with its headers, without an SMTP check', async () => {
      mockCreateChannel.mockResolvedValue({ ...WEBHOOK_CHANNEL, id: 7 })
      const wrapper = await mountWizard()
      await fillWebhook(wrapper)
      await wrapper.find('[data-testid="webhook-add-header"]').trigger('click')
      await headerInputs(wrapper, 'webhook-header-name')[0].setValue('Authorization')
      await headerInputs(wrapper, 'webhook-header-value')[0].setValue('Bearer new-token')
      await toReview(wrapper)
      expect(reviewBlock(wrapper, 'Delivery').text()).toContain('https://hooks.example.com/notify')
      await click(wrapper, 'Create channel')

      expect(mockValidateSmtp).not.toHaveBeenCalled()
      expect(mockCreateChannel).toHaveBeenCalledWith(
        expect.objectContaining({
          channel_type: 'webhook',
          config: expect.objectContaining({
            url: 'https://hooks.example.com/notify',
            headers: { Authorization: 'Bearer new-token' },
          }),
        }),
      )
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications/channels/7')
    })

    it('refuses to create a webhook channel with the same header twice', async () => {
      mockCreateChannel.mockResolvedValue({ ...WEBHOOK_CHANNEL, id: 7 })
      const wrapper = await mountWizard()
      await fillWebhook(wrapper)
      const addHeader = wrapper.find('[data-testid="webhook-add-header"]')
      await addHeader.trigger('click')
      await addHeader.trigger('click')
      await headerInputs(wrapper, 'webhook-header-name')[0].setValue('Authorization')
      await headerInputs(wrapper, 'webhook-header-name')[1].setValue('authorization')

      // The repeat blocks the Delivery step, and so the whole wizard.
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
      expect(footStatus(wrapper)).toContain('each header given once (Authorization)')
      expect(mockCreateChannel).not.toHaveBeenCalled()

      await headerInputs(wrapper, 'webhook-header-name')[1].setValue('X-Team')
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
      await toReview(wrapper)
      await click(wrapper, 'Create channel')
      expect(mockCreateChannel).toHaveBeenCalledOnce()
    })
  })

  describe('creating a web push channel', () => {
    beforeEach(() => {
      vi.mocked(getVapidPublicKey).mockResolvedValue({ key: 'AQID-_8', configured: true })
      mockCreateChannel.mockResolvedValue({ ...PUSH_CHANNEL, id: 9 })
    })

    afterEach(() => {
      unstubPushBrowser()
    })

    async function createPush(wrapper: ViewWrapper): Promise<void> {
      await chooseType(wrapper, 'web_push')
      await type(wrapper, 'Name', 'Laptop')
      await toReview(wrapper)
      await click(wrapper, 'Create channel')
    }

    it('subscribes this browser before creating the channel', async () => {
      const browser = stubPushBrowser('granted', [])
      const wrapper = await mountWizard()
      await createPush(wrapper)

      expect(browser.pushManager.subscribe).toHaveBeenCalledTimes(1)
      expect(vi.mocked(subscribePush)).toHaveBeenCalledWith({
        endpoint: 'https://push.example.com/new',
      })
      expect(mockCreateChannel).toHaveBeenCalledWith(
        expect.objectContaining({ name: 'Laptop', channel_type: 'web_push' }),
      )
      expect(vi.mocked(subscribePush).mock.invocationCallOrder[0]).toBeLessThan(
        mockCreateChannel.mock.invocationCallOrder[0],
      )
      expect(wrapper.vm.$router.currentRoute.value.path).toBe('/notifications/channels/9')
    })

    it('creates nothing when the browser refuses notifications', async () => {
      stubPushBrowser('denied', [])
      const wrapper = await mountWizard()
      await createPush(wrapper)

      expect(mockCreateChannel).not.toHaveBeenCalled()
      expect(wrapper.find('.form-error').text()).toContain('Notification permission was denied')
    })
  })

  describe('scope', () => {
    function scopeBoxes(wrapper: ViewWrapper) {
      return wrapper.findAll<HTMLInputElement>('.scope-item input[type="checkbox"]')
    }

    async function toScope(wrapper: ViewWrapper): Promise<void> {
      await fillEmail(wrapper, 'Scoped')
      await next(wrapper, 2)
    }

    it('offers the loaded repositories, agents and schedules', async () => {
      const wrapper = await mountWizard()
      await toScope(wrapper)

      expect(wrapper.text()).toContain('server-daily')
      expect(wrapper.text()).toContain('web-01')
      expect(wrapper.text()).toContain('Nightly web')
      expect(scopeBoxes(wrapper)).toHaveLength(3)
    })

    it('filters the scope list as you search', async () => {
      const wrapper = await mountWizard()
      await toScope(wrapper)

      const before = wrapper.findAll('.scope-item').length
      await wrapper.find('.scope-search').setValue('server-daily')
      await flushPromises()

      expect(wrapper.findAll('.scope-item').length).toBeLessThan(before)
    })

    // Scope is optional and additive: ticking a repository must narrow the
    // channel to it without dropping the agent or schedule selections.
    it('carries every scope kind through to the created channel', async () => {
      const wrapper = await mountWizard()
      await toScope(wrapper)

      for (const box of scopeBoxes(wrapper)) await box.setValue(true)
      await flushPromises()
      await next(wrapper, 2)
      expect(reviewBlock(wrapper, 'Scope').find('p').text()).not.toBe('All')
      await click(wrapper, 'Create channel')

      expect(mockCreateChannel).toHaveBeenCalledWith(
        expect.objectContaining({
          scope: { repo_ids: [7], agent_ids: [3], schedule_ids: [5] },
        }),
      )
    })

    it('drops an item that was ticked and then unticked', async () => {
      const wrapper = await mountWizard()
      await toScope(wrapper)

      await scopeBoxes(wrapper)[0].setValue(true)
      await scopeBoxes(wrapper)[0].setValue(false)
      await next(wrapper, 2)
      await click(wrapper, 'Create channel')

      expect(mockCreateChannel).toHaveBeenCalledWith(
        expect.objectContaining({ scope: { repo_ids: [] } }),
      )
    })

    it('still offers the wizard when the scope options fail to load', async () => {
      vi.mocked(apiClient.get).mockRejectedValue(new Error('scope down'))
      const wrapper = await mountWizard()
      await toScope(wrapper)

      expect(vi.mocked(logger.error).mock.calls.map((c) => c[0])).toContain(
        'loadScopeOptions failed',
      )
      expect(scopeBoxes(wrapper)).toHaveLength(0)
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
    })
  })

  describe('link', () => {
    async function toLink(wrapper: ViewWrapper): Promise<void> {
      await fillEmail(wrapper)
      await next(wrapper, 3)
    }

    function linkRadio(wrapper: ViewWrapper, target: string) {
      return wrapper.find<HTMLInputElement>(`#new-channel-link-${target}`)
    }

    it('starts on where the event happened', async () => {
      const wrapper = await mountWizard()
      await toLink(wrapper)

      expect(caption(wrapper)).toBe('Step 4 of 5 · Link')
      expect(linkRadio(wrapper, 'automatic').element.checked).toBe(true)
    })

    it('sends the picked link with the channel and names it in the review', async () => {
      const wrapper = await mountWizard()
      await toLink(wrapper)
      await linkRadio(wrapper, 'repository').setValue(true)
      await next(wrapper)

      expect(reviewBlock(wrapper, 'Link').find('p').text()).toBe('Repository')
      await click(wrapper, 'Create channel')
      expect(mockCreateChannel).toHaveBeenCalledWith(
        expect.objectContaining({ link: { target: 'repository' } }),
      )
    })

    it('will not continue to your own address until it is a web address', async () => {
      const wrapper = await mountWizard()
      await toLink(wrapper)
      await linkRadio(wrapper, 'custom').setValue(true)

      expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
      expect(footStatus(wrapper)).toBe('Needs an address to link to to continue.')

      await wrapper.find('#new-channel-link-url').setValue('status.example.com')
      expect(footStatus(wrapper)).toBe(
        'Needs an address starting with http:// or https:// to continue.',
      )

      await wrapper.find('#new-channel-link-url').setValue('https://status.example.com')
      expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
      await next(wrapper)
      expect(reviewBlock(wrapper, 'Link').find('p').text()).toBe('https://status.example.com')

      await click(wrapper, 'Create channel')
      expect(mockCreateChannel).toHaveBeenCalledWith(
        expect.objectContaining({
          link: { target: 'custom', url: 'https://status.example.com' },
        }),
      )
    })
  })

  describe('review', () => {
    it('summarises every step', async () => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await toReview(wrapper)

      expect(wrapper.findAll('.review-block .group-label').map((l) => l.text())).toEqual([
        'Delivery',
        'Events',
        'Scope',
        'Link',
      ])
      const delivery = reviewBlock(wrapper, 'Delivery')
      const terms = delivery.findAll('dt').map((t) => t.text())
      const values = delivery.findAll('dd').map((d) => d.text())
      expect(Object.fromEntries(terms.map((t, i) => [t, values[i]]))).toEqual({
        Name: 'Ops Alerts',
        Type: 'Email',
        To: 'admin@example.com',
        State: 'On',
      })
      expect(reviewBlock(wrapper, 'Scope').find('p').text()).toBe('All')
      expect(reviewBlock(wrapper, 'Link').find('p').text()).toBe('Where the event happened')
    })

    it.each([
      ['Delivery', 'Step 1 of 5 · Delivery'],
      ['Events', 'Step 2 of 5 · Events'],
      ['Scope', 'Step 3 of 5 · Scope'],
      ['Link', 'Step 4 of 5 · Link'],
    ])('goes back to the %s step from its review block', async (label, expected) => {
      const wrapper = await mountWizard()
      await fillEmail(wrapper)
      await toReview(wrapper)

      await reviewBlock(wrapper, label).find('button').trigger('click')
      await flushPromises()

      expect(caption(wrapper)).toBe(expected)
    })
  })
})
