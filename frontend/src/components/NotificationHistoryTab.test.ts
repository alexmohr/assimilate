// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import NotificationHistoryTab from './NotificationHistoryTab.vue'
import type {
  NotificationChannel,
  NotificationDelivery,
  NotificationEventType,
} from '../types/notifications'

const CHANNELS = [
  { id: 1, name: 'Ops Email' },
  { id: 2, name: 'Slack Hook' },
] as unknown as NotificationChannel[]

function delivery(overrides: Partial<NotificationDelivery> = {}): NotificationDelivery {
  return {
    id: 1,
    channel_id: 1,
    event_type: 'backup_failed',
    status: 'sent',
    error_message: null,
    payload: { repo: 'main' },
    attempted_at: '2026-01-01T02:00:00Z',
    ...overrides,
  } as unknown as NotificationDelivery
}

const eventTypeLabel = (event: NotificationEventType): string => {
  const words = event.split('_')
  return [words[0].charAt(0).toUpperCase() + words[0].slice(1), ...words.slice(1)].join(' ')
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(NotificationHistoryTab, {
    props: { deliveries: [delivery()], channels: CHANNELS, eventTypeLabel, ...props },
  })
}

describe('NotificationHistoryTab', () => {
  it('shows the shared empty state when nothing has been delivered', () => {
    const wrapper = mount({ deliveries: [] })
    expect(wrapper.text()).toContain('No deliveries yet')
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('names the channel a delivery went to', () => {
    const wrapper = mount({
      deliveries: [delivery({ id: 1, channel_id: 1 }), delivery({ id: 2, channel_id: 2 })],
    })
    expect(wrapper.text()).toContain('Ops Email')
    expect(wrapper.text()).toContain('Slack Hook')
  })

  it('falls back to the channel id when the channel is gone', () => {
    const wrapper = mount({ deliveries: [delivery({ channel_id: 77 })] })
    expect(wrapper.text()).toContain('77')
  })

  it('labels the event type through the callers formatter', () => {
    expect(mount().text()).toContain('Backup failed')
  })

  it('tones the status by outcome', () => {
    expect(mount().find('.delivery-status').classes()).toContain('status-sent')
    expect(
      mount({ deliveries: [delivery({ status: 'failed' })] })
        .find('.delivery-status')
        .classes(),
    ).toContain('status-failed')
    expect(
      mount({ deliveries: [delivery({ status: 'pending' })] })
        .find('.delivery-status')
        .classes(),
    ).toContain('status-pending')
  })

  it('expands a row to show the payload, and collapses it again', async () => {
    const wrapper = mount()
    expect(wrapper.find('.detail-row').exists()).toBe(false)

    await wrapper.find('.delivery-row').trigger('click')
    expect(wrapper.find('.detail-row').exists()).toBe(true)
    expect(wrapper.find('.detail-pre').text()).toContain('"repo": "main"')

    await wrapper.find('.delivery-row').trigger('click')
    expect(wrapper.find('.detail-row').exists()).toBe(false)
  })

  it('expands one row at a time', async () => {
    const wrapper = mount({ deliveries: [delivery({ id: 1 }), delivery({ id: 2 })] })
    await wrapper.findAll('.delivery-row')[0].trigger('click')
    await wrapper.findAll('.delivery-row')[1].trigger('click')
    expect(wrapper.findAll('.detail-row')).toHaveLength(1)
  })

  it('shows the error block only for a delivery that failed', async () => {
    const clean = mount()
    await clean.find('.delivery-row').trigger('click')
    expect(clean.find('.error-pre').exists()).toBe(false)

    const failed = mount({
      deliveries: [delivery({ status: 'failed', error_message: 'connection refused' })],
    })
    await failed.find('.delivery-row').trigger('click')
    expect(failed.find('.error-pre').text()).toBe('connection refused')
  })

  it('labels every cell for the stacked mobile layout', () => {
    const labels = mount()
      .findAll('.delivery-row td[data-label]')
      .map((c) => c.attributes('data-label'))
    expect(labels).toEqual(['Channel', 'Event', 'Status', 'Error', 'Time'])
  })
})
