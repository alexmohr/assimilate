// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises, type DOMWrapper } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { pushWs, resetWsMock } from '../test-utils/wsMock'
import {
  EMAIL_CHANNEL,
  MOCK_RULES,
  PUSH_CHANNEL,
  WEBHOOK_CHANNEL,
  delivery,
  stubPushBrowser,
  unstubPushBrowser,
} from '../test-utils/notificationFixtures'
import NotificationsView from './NotificationsView.vue'

vi.mock('../api/notifications', () => ({
  listChannels: vi.fn(),
  listRules: vi.fn(),
  listDeliveries: vi.fn(),
  getVapidPublicKey: vi.fn(),
  subscribePush: vi.fn(),
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
  listChannels,
  listRules,
  listDeliveries,
  getVapidPublicKey,
  subscribePush,
} from '../api/notifications'
import { logger } from '../utils/logger'
import type {
  NotificationChannelResponse as NotificationChannel,
  NotificationDeliveryResponse as NotificationDelivery,
} from '../types/generated'

const mockListChannels = vi.mocked(listChannels)
const mockListRules = vi.mocked(listRules)
const mockListDeliveries = vi.mocked(listDeliveries)
const mockGetVapidPublicKey = vi.mocked(getVapidPublicKey)

function setupDefaultMocks(): void {
  mockListChannels.mockResolvedValue([WEBHOOK_CHANNEL, EMAIL_CHANNEL])
  mockListRules.mockResolvedValue(MOCK_RULES)
  mockListDeliveries.mockResolvedValue([])
  mockGetVapidPublicKey.mockResolvedValue({ key: '', configured: false })
}

type ViewWrapper = ReturnType<typeof renderWithPlugins>

async function mountView(route = '/notifications'): Promise<ViewWrapper> {
  const wrapper = renderWithPlugins(NotificationsView)
  await wrapper.vm.$router.push(route)
  await flushPromises()
  return wrapper
}

/** The channel rows, in the order the list shows them. */
function channelRows(wrapper: ViewWrapper): DOMWrapper<Element>[] {
  return wrapper.findAll('.list-row[data-channel]')
}

function channelRow(wrapper: ViewWrapper, name: string): DOMWrapper<Element> {
  const row = channelRows(wrapper).find((r) => r.attributes('data-channel') === name)
  if (!row) throw new Error(`no channel row for "${name}"`)
  return row
}

function tab(wrapper: ViewWrapper, label: string): DOMWrapper<Element> {
  const match = wrapper.findAll('[role="tab"]').find((t) => t.text().includes(label))
  if (!match) throw new Error(`no "${label}" tab`)
  return match
}

async function openHistory(wrapper: ViewWrapper): Promise<void> {
  await tab(wrapper, 'History').trigger('click')
  await flushPromises()
}

function currentRoute(wrapper: ViewWrapper): {
  path: string
  query: Record<string, unknown>
} {
  return wrapper.vm.$router.currentRoute.value
}

describe('NotificationsView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetWsMock()
    setupDefaultMocks()
  })

  it('renders page title', async () => {
    const wrapper = await mountView()
    expect(wrapper.find('.page-title').text()).toBe('Notifications')
  })

  it('renders Channels and History tabs', async () => {
    const wrapper = await mountView()
    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs.map((t) => t.text().replace(/\s+/g, ' '))).toEqual(['Channels 2', 'History'])
    expect(tabs[0].attributes('aria-selected')).toBe('true')
  })

  it('renders webhook channel card', async () => {
    const wrapper = await mountView()
    expect(channelRow(wrapper, 'Ops Webhook').find('.list-row-name').text()).toBe('Ops Webhook')
  })

  it('renders email channel card', async () => {
    const wrapper = await mountView()
    expect(channelRow(wrapper, 'Ops Email').find('.list-row-name').text()).toBe('Ops Email')
  })

  it('renders channel type labels', async () => {
    const wrapper = await mountView()
    expect(channelRow(wrapper, 'Ops Webhook').find('.list-row-sub').text()).toContain('Webhook')
    expect(channelRow(wrapper, 'Ops Email').find('.list-row-sub').text()).toContain('Email')
  })

  // A row carries no buttons of its own: everything a channel can do lives on
  // its own page, which the row opens.
  it('opens each channel on its own page rather than acting from the row', async () => {
    const wrapper = await mountView()
    const rows = channelRows(wrapper)
    expect(rows).toHaveLength(2)
    for (const row of rows) {
      expect(row.element.tagName).toBe('A')
      expect(row.findAll('button')).toHaveLength(0)
      expect(row.findComponent({ name: 'ToggleSwitch' }).exists()).toBe(false)
    }
    expect(channelRow(wrapper, 'Ops Webhook').attributes('href')).toBe('/notifications/channels/1')
    expect(channelRow(wrapper, 'Ops Email').attributes('href')).toBe('/notifications/channels/2')
  })

  it('summarises each channel on its row without its secrets', async () => {
    mockListChannels.mockResolvedValue([
      {
        ...WEBHOOK_CHANNEL,
        config: { url: 'https://user:hunter2@hooks.example.com/notify?token=abc' },
      } as NotificationChannel,
      { ...EMAIL_CHANNEL, enabled: false, scope: { repo_ids: [1, 2] } } as NotificationChannel,
    ])
    const wrapper = await mountView()

    const hook = channelRow(wrapper, 'Ops Webhook')
    expect(hook.find('.list-row-sub').text()).toBe('Webhook · hooks.example.com')
    expect(hook.text()).not.toContain('hunter2')
    expect(hook.text()).not.toContain('token')
    expect(hook.find('.channel-events').text()).toBe('1 of 14 events')
    expect(hook.find('.channel-scope').text()).toBe('All')
    expect(hook.find('.channel-delivery').text()).toBe('Nothing sent yet')
    // An enabled channel says nothing in its State column; only a paused one does.
    expect(hook.find('.list-row-value.muted').text()).toBe('')

    const email = channelRow(wrapper, 'Ops Email')
    expect(email.find('.list-row-sub').text()).toBe('Email · admin@example.com')
    expect(email.find('.channel-events').text()).toBe('No events')
    expect(email.find('.channel-scope').text()).toBe('2 repos')
    expect(email.find('.list-row-value.muted').text()).toBe('Paused')
    expect(email.find('.channel-summary').text()).toBe('No events · 2 repos · Paused')
  })

  it('counts the channels on their tab', async () => {
    const wrapper = await mountView()
    expect(tab(wrapper, 'Channels').find('.tab-count').text()).toBe('2')
  })

  it('opens the new-channel wizard page from New', async () => {
    const wrapper = await mountView()
    const newBtn = wrapper.findAll('.page-header button').find((b) => b.text().includes('New'))
    expect(newBtn).toBeDefined()
    await newBtn!.trigger('click')
    await flushPromises()
    expect(currentRoute(wrapper).path).toBe('/notifications/new')
  })

  it('offers New only on the Channels tab', async () => {
    const wrapper = await mountView()
    await openHistory(wrapper)
    expect(wrapper.find('.page-header button').exists()).toBe(false)
  })

  it('shows empty state when no channels exist, with a way to make one', async () => {
    mockListChannels.mockResolvedValue([])
    mockListRules.mockResolvedValue([])
    const wrapper = await mountView()
    expect(wrapper.find('.empty-title').text()).toBe('No notification channels')

    await wrapper.find('.empty-action').trigger('click')
    await flushPromises()
    expect(currentRoute(wrapper).path).toBe('/notifications/new')
  })

  it('shows the load error in place of the channels', async () => {
    mockListChannels.mockRejectedValue(new Error('boom'))
    const wrapper = await mountView()
    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(channelRows(wrapper)).toHaveLength(0)
  })

  it('opens the History tab from the URL, so a link can land on it', async () => {
    const wrapper = await mountView('/notifications?tab=history')
    expect(tab(wrapper, 'History').attributes('aria-selected')).toBe('true')
    expect(wrapper.find('.empty-title').text()).toBe('No deliveries yet')
  })

  it('keeps the open tab in the URL', async () => {
    const wrapper = await mountView()
    await openHistory(wrapper)
    expect(currentRoute(wrapper).query.tab).toBe('history')
  })

  it('shows empty delivery history message on History tab', async () => {
    const wrapper = await mountView()
    await openHistory(wrapper)
    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.find('.empty-title').text()).toBe('No deliveries yet')
  })

  it('expands a delivery row to reveal the full error and payload', async () => {
    mockListDeliveries.mockResolvedValue([
      {
        id: 1,
        channel_id: WEBHOOK_CHANNEL.id,
        event_type: 'backup_failed',
        payload: { hostname: 'web-01', repo_name: 'daily-backup' },
        status: 'failed',
        error_message: 'connection refused',
        attempted_at: '2026-01-15T03:00:12Z',
      } as unknown as NotificationDelivery,
    ])

    const wrapper = await mountView()
    await openHistory(wrapper)

    expect(wrapper.text()).not.toContain('web-01')

    const deliveryRow = wrapper.find('[data-testid="delivery-history"] .log-row-summary')
    expect(deliveryRow.exists()).toBe(true)
    await deliveryRow.trigger('click')
    await flushPromises()

    expect(wrapper.find('.log-row-detail').exists()).toBe(true)
    expect(wrapper.find('.log-row-detail .error-pre').text()).toBe('connection refused')
    expect(wrapper.text()).toContain('web-01')
    expect(wrapper.text()).toContain('daily-backup')

    await deliveryRow.trigger('click')
    await flushPromises()
    expect(wrapper.find('.log-row-detail').exists()).toBe(false)
  })

  it('adds a live delivery to the top of the history, keeping the newest twenty', async () => {
    mockListDeliveries.mockResolvedValue(
      Array.from({ length: 20 }, (_, i) => delivery(i + 1, 1, 'sent', i + 1)),
    )
    const wrapper = await mountView()
    await openHistory(wrapper)
    expect(wrapper.findAll('[data-testid="delivery-history"] .log-row')).toHaveLength(20)

    pushWs('NotificationDelivery', delivery(99, 2, 'failed', 0, 'live failure'))
    await flushPromises()

    const rows = wrapper.findAll('[data-testid="delivery-history"] .log-row')
    expect(rows).toHaveLength(20)
    expect(rows[0].find('.log-row-subtitle').text()).toBe('Ops Email')
    expect(rows[0].find('.log-row-facts').text()).toBe('live failure')
  })

  it('still lists the channels when the history fails to load', async () => {
    mockListDeliveries.mockRejectedValue(new Error('history down'))
    const wrapper = await mountView()

    expect(vi.mocked(logger.error).mock.calls.map((c) => c[0])).toContain('loadDeliveries failed')
    expect(channelRows(wrapper)).toHaveLength(2)
  })

  describe('delivery health', () => {
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
      const emailRow = channelRow(wrapper, 'Ops Email')
      const email = emailRow.find('.channel-delivery')
      expect(email.classes()).toContain('channel-delivery--danger')
      expect(email.text()).toBe('Failed 2h ago · SMTP auth failed: 535 bad credentials')
      expect(emailRow.find('.status-dot').classes()).toContain('status-dot--danger')
      const hookRow = channelRow(wrapper, 'Ops Webhook')
      const hook = hookRow.find('.channel-delivery')
      expect(hook.classes()).not.toContain('channel-delivery--danger')
      expect(hook.text()).toBe('Sent 1d ago')
      expect(hookRow.find('.status-dot').classes()).toContain('status-dot--success')
    })

    // The call-out leads to the failing channel's own history.
    it('links the failure notice to the failing channel’s history', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      const view = wrapper.findAll('.notice-bar a').find((b) => b.text() === 'View history')
      expect(view?.attributes('href')).toBe('/notifications/channels/2?section=history')
    })

    it('narrows the History tab to the channel picked in its filter', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()
      await openHistory(wrapper)

      expect(wrapper.findAll('[data-testid="delivery-history"] .log-row')).toHaveLength(
        HISTORY.length,
      )
      expect(wrapper.find<HTMLSelectElement>('.toolbar select').element.value).toBe('')

      await wrapper.find<HTMLSelectElement>('.toolbar select').setValue('2')
      await flushPromises()
      expect(wrapper.findAll('[data-testid="delivery-history"] .log-row')).toHaveLength(3)
    })

    it('keeps the channel picked in the history filter across tab switches', async () => {
      mockListDeliveries.mockResolvedValue(HISTORY)
      const wrapper = await mountView()

      await openHistory(wrapper)
      await wrapper.find<HTMLSelectElement>('.toolbar select').setValue('2')
      await flushPromises()
      expect(wrapper.findAll('[data-testid="delivery-history"] .log-row')).toHaveLength(3)

      // The pick lives on the page, not in the tab, so leaving the tab and
      // coming back finds the history still narrowed to that channel.
      await tab(wrapper, 'Channels').trigger('click')
      await flushPromises()
      await openHistory(wrapper)
      expect(wrapper.find<HTMLSelectElement>('.toolbar select').element.value).toBe('2')
      expect(wrapper.findAll('[data-testid="delivery-history"] .log-row')).toHaveLength(3)
    })

    it('says nothing was sent before anything was', async () => {
      const wrapper = await mountView()
      expect(channelRow(wrapper, 'Ops Webhook').find('.channel-delivery').text()).toBe(
        'Nothing sent yet',
      )
      expect(channelRow(wrapper, 'Ops Webhook').find('.status-dot').classes()).toEqual([
        'status-dot',
      ])
    })
  })

  describe('web push', () => {
    beforeEach(() => {
      mockListChannels.mockResolvedValue([PUSH_CHANNEL])
      mockGetVapidPublicKey.mockResolvedValue({ key: 'AQID-_8', configured: true })
    })

    afterEach(() => {
      unstubPushBrowser()
    })

    it('re-registers the browser subscription it finds on load', async () => {
      stubPushBrowser('granted', [])

      await mountView()

      expect(vi.mocked(subscribePush)).toHaveBeenCalledWith({
        endpoint: 'https://push.example.com/old',
      })
    })

    it('registers nothing when the server has no push keys', async () => {
      const browser = stubPushBrowser('granted', [])
      mockGetVapidPublicKey.mockResolvedValue({ key: '', configured: false })

      await mountView()

      expect(browser.pushManager.getSubscription).not.toHaveBeenCalled()
      expect(vi.mocked(subscribePush)).not.toHaveBeenCalled()
    })

    it('registers nothing when this browser holds no subscription', async () => {
      const browser = stubPushBrowser('granted', [])
      browser.pushManager.getSubscription.mockResolvedValue(null)

      await mountView()

      expect(vi.mocked(subscribePush)).not.toHaveBeenCalled()
    })

    it('registers nothing in a browser without web push', async () => {
      await mountView()
      expect(vi.mocked(subscribePush)).not.toHaveBeenCalled()
    })

    it('keeps the page up when the push key cannot be read', async () => {
      mockGetVapidPublicKey.mockRejectedValue(new Error('down'))
      const wrapper = await mountView()

      expect(vi.mocked(logger.error).mock.calls.map((c) => c[0])).toContain('loadPushStatus failed')
      expect(channelRows(wrapper)).toHaveLength(1)
    })
  })
})
