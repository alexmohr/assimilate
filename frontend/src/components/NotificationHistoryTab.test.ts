// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import NotificationHistoryTab from './NotificationHistoryTab.vue'
import type {
  EventType as NotificationEventType,
  NotificationChannelResponse as NotificationChannel,
  NotificationDeliveryResponse as NotificationDelivery,
} from '../types/generated'

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
    expect(wrapper.find('.log-row').exists()).toBe(false)
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

  it('tones the status dot by outcome', () => {
    const dot = (status: string): string[] =>
      mount({ deliveries: [delivery({ status } as Partial<NotificationDelivery>)] })
        .find('.log-row-dot')
        .classes()
    expect(dot('sent')).toContain('status-dot--success')
    expect(dot('failed')).toContain('status-dot--danger')
    expect(dot('pending')).toContain('status-dot--warning')
  })

  it('says the outcome on the row, or the error of a failed attempt', () => {
    expect(mount().find('.log-row-facts').text()).toBe('Sent')
    expect(
      mount({ deliveries: [delivery({ status: 'pending' })] })
        .find('.log-row-facts')
        .text(),
    ).toBe('Pending')
    expect(
      mount({
        deliveries: [
          delivery({ status: 'failed', error_message: 'connection refused\nat line 2' }),
        ],
      })
        .find('.log-row-facts')
        .text(),
    ).toBe('connection refused')
    expect(
      mount({ deliveries: [delivery({ status: 'failed', error_message: null })] })
        .find('.log-row-facts')
        .text(),
    ).toBe('Failed')
  })

  it('expands a row to show the payload, and collapses it again', async () => {
    const wrapper = mount()
    expect(wrapper.find('.log-row-detail').exists()).toBe(false)

    await wrapper.find('.log-row-summary').trigger('click')
    expect(wrapper.find('.log-row-detail').exists()).toBe(true)
    expect(wrapper.find('.log-row-summary').attributes('aria-expanded')).toBe('true')
    expect(wrapper.find('.detail-pre').text()).toContain('"repo": "main"')

    await wrapper.find('.log-row-summary').trigger('click')
    expect(wrapper.find('.log-row-detail').exists()).toBe(false)
  })

  it('expands one row at a time', async () => {
    const wrapper = mount({ deliveries: [delivery({ id: 1 }), delivery({ id: 2 })] })
    await wrapper.findAll('.log-row-summary')[0].trigger('click')
    await wrapper.findAll('.log-row-summary')[1].trigger('click')
    expect(wrapper.findAll('.log-row-detail')).toHaveLength(1)
  })

  it('shows the error block only for a delivery that failed', async () => {
    const clean = mount()
    await clean.find('.log-row-summary').trigger('click')
    expect(clean.find('.error-pre').exists()).toBe(false)

    const failed = mount({
      deliveries: [delivery({ status: 'failed', error_message: 'connection refused' })],
    })
    await failed.find('.log-row-summary').trigger('click')
    expect(failed.find('.error-pre').text()).toBe('connection refused')
  })

  it('names the channel, status and time in the open detail', async () => {
    const wrapper = mount({ deliveries: [delivery({ status: 'failed', error_message: 'x' })] })
    await wrapper.find('.log-row-summary').trigger('click')
    const terms = wrapper.findAll('.log-row-detail dt').map((t) => t.text())
    expect(terms).toEqual(['Time', 'Channel', 'Status'])
    const values = wrapper.findAll('.log-row-detail dd').map((d) => d.text())
    expect(values[1]).toBe('Ops Email')
    expect(values[2]).toBe('Failed')
  })

  describe('channel filter', () => {
    const TWO = [delivery({ id: 1, channel_id: 1 }), delivery({ id: 2, channel_id: 2 })]

    it('shows every channel when no filter is set', () => {
      const wrapper = mount({ deliveries: TWO })
      expect(wrapper.findAll('.log-row')).toHaveLength(2)
      expect(wrapper.find<HTMLSelectElement>('select').element.value).toBe('')
    })

    it('narrows the log to the channel it is given', () => {
      const wrapper = mount({ deliveries: TWO, channelId: 2 })
      const rows = wrapper.findAll('.log-row')
      expect(rows).toHaveLength(1)
      expect(rows[0].text()).toContain('Slack Hook')
      expect(wrapper.find<HTMLSelectElement>('select').element.value).toBe('2')
    })

    it('asks for a channel, or for all of them, when the picker changes', async () => {
      const wrapper = mount({ deliveries: TWO })
      await wrapper.find('select').setValue('1')
      await wrapper.find('select').setValue('')
      expect(wrapper.emitted('update:channelId')).toEqual([[1], [null]])
    })

    it('says so, and offers every channel back, when the channel sent nothing', async () => {
      const wrapper = mount({ deliveries: [delivery({ channel_id: 1 })], channelId: 2 })
      expect(wrapper.find('.log-row').exists()).toBe(false)
      expect(wrapper.text()).toContain('Nothing sent through this channel yet')
      await wrapper.find('.empty-state button').trigger('click')
      expect(wrapper.emitted('update:channelId')).toEqual([[null]])
    })

    it('has no picker with only one channel to pick', () => {
      const wrapper = mount({ channels: [CHANNELS[0]] })
      expect(wrapper.find('select').exists()).toBe(false)
    })
  })
})
