// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  channelFailureSummaries,
  channelTarget,
  deliveryTone,
  lastDeliveryLabel,
  latestDeliveryByChannel,
  recentDeliveries,
  shortError,
  sortChannelsByTrouble,
  webhookHost,
} from './notificationChannels'
import type {
  NotificationChannelResponse as Channel,
  NotificationDeliveryResponse as Delivery,
} from '../types/generated'

const NOW = new Date('2026-03-10T12:00:00Z')

function channel(id: number, overrides: Record<string, unknown> = {}): Channel {
  return {
    id,
    name: `Channel ${id}`,
    channel_type: 'webhook',
    config: { url: 'https://hooks.example.com/x' },
    has_password: false,
    webhook_headers: [],
    enabled: true,
    scope: {},
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as unknown as Channel
}

function delivery(overrides: Partial<Delivery> = {}): Delivery {
  return {
    id: 1,
    channel_id: 1,
    event_type: 'backup_failed',
    status: 'sent',
    error_message: null,
    payload: {},
    attempted_at: '2026-03-10T11:00:00Z',
    ...overrides,
  } as unknown as Delivery
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
})

afterEach(() => {
  vi.useRealTimers()
})

describe('channelFailureSummaries', () => {
  it('is empty when nothing failed', () => {
    const out = channelFailureSummaries([channel(1)], [delivery(), delivery({ id: 2 })])
    expect(out).toEqual([])
  })

  it('counts a channel failures against all of its recent deliveries', () => {
    const deliveries = [
      delivery({ id: 1, status: 'sent', attempted_at: '2026-03-10T08:00:00Z' }),
      delivery({
        id: 2,
        status: 'failed',
        error_message: 'older error',
        attempted_at: '2026-03-10T09:00:00Z',
      }),
      delivery({
        id: 3,
        status: 'failed',
        error_message: 'newest error',
        attempted_at: '2026-03-10T10:00:00Z',
      }),
      delivery({ id: 4, channel_id: 2, status: 'sent' }),
    ]
    const out = channelFailureSummaries([channel(1), channel(2)], deliveries)
    expect(out).toHaveLength(1)
    expect(out[0].channel.id).toBe(1)
    expect(out[0].failed).toBe(2)
    expect(out[0].total).toBe(3)
    expect(out[0].lastError).toBe('newest error')
  })

  it('reports each failing channel, and none for a deleted one', () => {
    const deliveries = [
      delivery({ id: 1, channel_id: 1, status: 'failed' }),
      delivery({ id: 2, channel_id: 2, status: 'failed', error_message: 'x' }),
      delivery({ id: 3, channel_id: 99, status: 'failed' }),
    ]
    const out = channelFailureSummaries([channel(1), channel(2)], deliveries)
    expect(out.map((f) => f.channel.id)).toEqual([1, 2])
    expect(out[0].lastError).toBeNull()
  })
})

describe('sortChannelsByTrouble', () => {
  it('puts failing channels first and keeps the rest in order', () => {
    const sorted = sortChannelsByTrouble([channel(1), channel(2), channel(3)], new Set([3]))
    expect(sorted.map((c) => c.id)).toEqual([3, 1, 2])
  })
})

describe('latest and recent deliveries', () => {
  const deliveries = [
    delivery({ id: 1, attempted_at: '2026-03-10T08:00:00Z' }),
    delivery({ id: 2, attempted_at: '2026-03-10T10:00:00Z' }),
    delivery({ id: 3, channel_id: 2, attempted_at: '2026-03-10T09:00:00Z' }),
  ]

  it('picks each channel newest delivery whatever the order', () => {
    const latest = latestDeliveryByChannel(deliveries)
    expect(latest.get(1)?.id).toBe(2)
    expect(latest.get(2)?.id).toBe(3)
    expect(latest.has(5)).toBe(false)
  })

  it('takes the newest few across every channel', () => {
    expect(recentDeliveries(deliveries, 2).map((d) => d.id)).toEqual([2, 3])
  })
})

describe('channelTarget', () => {
  it('shows only the host of a webhook URL, never its path or credentials', () => {
    const c = channel(1, {
      config: { url: 'https://user:secret@hooks.slack.com/services/T000/B000/XXXX?token=abc' },
    })
    expect(channelTarget(c)).toBe('hooks.slack.com')
  })

  it('does not echo an unparseable URL back', () => {
    expect(webhookHost('not a url with token=abc')).toBe('Invalid URL')
  })

  it('names the first email recipient and how many more there are', () => {
    const one = channel(1, { channel_type: 'email', config: { to_addresses: ['a@x.io'] } })
    const three = channel(1, {
      channel_type: 'email',
      config: { to_addresses: ['a@x.io', 'b@x.io', 'c@x.io'] },
    })
    const none = channel(1, { channel_type: 'email', config: { to_addresses: [] } })
    expect(channelTarget(one)).toBe('a@x.io')
    expect(channelTarget(three)).toBe('a@x.io +2 more')
    expect(channelTarget(none)).toBe('No recipients')
  })

  it('describes a web push channel without an address', () => {
    expect(channelTarget(channel(1, { channel_type: 'web_push', config: {} }))).toBe(
      'Subscribed browsers',
    )
  })
})

describe('delivery labels', () => {
  it('tones each status', () => {
    expect(deliveryTone('sent')).toBe('success')
    expect(deliveryTone('failed')).toBe('danger')
    expect(deliveryTone('pending')).toBe('warning')
  })

  it('cuts a long or multi-line error to one short line', () => {
    expect(shortError('first line\nsecond line')).toBe('first line')
    const long = shortError('x'.repeat(200))
    expect(long.length).toBeLessThanOrEqual(60)
    expect(long.endsWith('...')).toBe(true)
    expect(shortError(null)).toBe('')
  })

  it('says when the last delivery went out and how it went', () => {
    expect(lastDeliveryLabel(undefined)).toBe('Nothing sent yet')
    expect(lastDeliveryLabel(delivery({ attempted_at: '2026-03-09T12:00:00Z' }))).toBe(
      'Sent 1d ago',
    )
    expect(
      lastDeliveryLabel(
        delivery({
          status: 'failed',
          error_message: 'timeout',
          attempted_at: '2026-03-10T10:00:00Z',
        }),
      ),
    ).toBe('Failed 2h ago · timeout')
    expect(
      lastDeliveryLabel(delivery({ status: 'failed', attempted_at: '2026-03-10T10:00:00Z' })),
    ).toBe('Failed 2h ago')
    expect(
      lastDeliveryLabel(delivery({ status: 'pending', attempted_at: '2026-03-10T11:00:00Z' })),
    ).toBe('Pending 1h ago')
  })
})
