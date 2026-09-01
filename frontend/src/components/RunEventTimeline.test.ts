// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RunEventTimeline from './RunEventTimeline.vue'
import type { RunEventResponse } from '../types/generated'

function event(over: Partial<RunEventResponse> = {}): RunEventResponse {
  return {
    id: 1,
    run_id: 'run-123',
    target: 'source',
    event_type: 'reachability_check',
    message: 'Checked agent -- no response',
    occurred_at: '2026-06-01T03:00:00Z',
    ...over,
  }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RunEventTimeline, {
    props: {
      events: [event()],
      sourceLabel: 'web-01',
      repositoryLabel: 'borg@192.168.1.50',
      ...props,
    },
  })
}

describe('RunEventTimeline', () => {
  it('renders one row per event with its message', () => {
    const wrapper = mount({
      events: [
        event({ id: 1, message: 'Checked agent -- no response' }),
        event({ id: 2, message: 'Sent Wake-on-LAN packet to 3C:97:0E:2B:9A:44' }),
      ],
    })
    const rows = wrapper.findAll('.run-timeline-row')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.text()).toContain('Checked agent -- no response')
    expect(rows[1]!.text()).toContain('Sent Wake-on-LAN packet')
  })

  it('sorts events chronologically regardless of input order', () => {
    const wrapper = mount({
      events: [
        event({ id: 1, message: 'second', occurred_at: '2026-06-01T03:00:05Z' }),
        event({ id: 2, message: 'first', occurred_at: '2026-06-01T03:00:00Z' }),
      ],
    })
    const messages = wrapper.findAll('.run-timeline-msg').map((m) => m.text())
    expect(messages).toEqual(['first', 'second'])
  })

  it('labels a source-target row with the agent hostname', () => {
    const wrapper = mount({ events: [event({ target: 'source' })] })
    expect(wrapper.find('.run-timeline-eyebrow--source').text()).toContain('web-01')
  })

  it('labels a repository-target row with the repository label', () => {
    const wrapper = mount({ events: [event({ target: 'repository' })] })
    expect(wrapper.find('.run-timeline-eyebrow--repository').text()).toContain('borg@192.168.1.50')
  })

  it('does not draw a connecting line after the last row', () => {
    const wrapper = mount({
      events: [event({ id: 1 }), event({ id: 2, occurred_at: '2026-06-01T03:00:05Z' })],
    })
    expect(wrapper.findAll('.run-timeline-line')).toHaveLength(1)
  })
})
