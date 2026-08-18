// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import RunHistoryStrip, { type RunHistoryEntry } from './RunHistoryStrip.vue'

function run(overrides: Partial<RunHistoryEntry> = {}): RunHistoryEntry {
  return {
    id: 1,
    startedAt: '2026-06-01T02:00:00Z',
    durationSecs: 600,
    status: 'success',
    ...overrides,
  }
}

describe('RunHistoryStrip', () => {
  it('renders empty stub bars and a "No runs yet" caption when there are no runs', () => {
    const wrapper = mount(RunHistoryStrip, { props: { runs: [] } })
    expect(wrapper.findAll('.run-bar-empty')).toHaveLength(10)
    expect(wrapper.text()).toContain('No runs yet')
  })

  it('draws one bar per run and reports the count and duration range', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, startedAt: '2026-06-01T01:00:00Z', durationSecs: 300 }),
          run({ id: 2, startedAt: '2026-06-01T02:00:00Z', durationSecs: 900 }),
        ],
      },
    })
    expect(wrapper.findAll('.run-bar')).toHaveLength(2)
    expect(wrapper.text()).toContain('2 runs · 5m 0s-15m 0s')
  })

  it('draws a failed run at full height instead of proportional to its (short) duration', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, durationSecs: 1200, status: 'success' }),
          run({ id: 2, durationSecs: 5, status: 'failed' }),
        ],
      },
    })
    const bars = wrapper.findAll('.run-bar')
    const failedBar = bars.find((b) => b.classes().includes('run-bar-danger'))
    expect(failedBar!.attributes('style')).toContain('height: 100%')
  })

  it('reports the failed count instead of a duration range when any run failed', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [run({ id: 1, status: 'success' }), run({ id: 2, status: 'failed' })],
      },
    })
    expect(wrapper.text()).toContain('2 runs · 1 failed')
  })

  it('shows only the most recent maxBars runs, oldest first', () => {
    const runs = Array.from({ length: 15 }, (_, i) =>
      run({ id: i, startedAt: `2026-06-01T${String(i).padStart(2, '0')}:00:00Z` }),
    )
    const wrapper = mount(RunHistoryStrip, { props: { runs, maxBars: 10 } })
    const bars = wrapper.findAll('.run-bar')
    expect(bars).toHaveLength(10)
    // Ids 0-4 were dropped; the surviving ten (5..14) render oldest first.
    expect(bars.map((b) => b.attributes('data-run-id'))).toEqual(
      Array.from({ length: 10 }, (_, i) => String(i + 5)),
    )
  })

  it('colors a warning run distinctly from success and failure', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: { runs: [run({ status: 'warning' })] },
    })
    expect(wrapper.find('.run-bar-warning').exists()).toBe(true)
  })

  it('colors a cancelled run distinctly from a failed one and excludes it from the failed count', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, status: 'success' }),
          run({ id: 2, durationSecs: 5, status: 'cancelled' }),
        ],
      },
    })
    const bars = wrapper.findAll('.run-bar')
    const cancelledBar = bars.find((b) => b.classes().includes('run-bar-neutral'))
    expect(cancelledBar).toBeTruthy()
    expect(wrapper.find('.run-bar-danger').exists()).toBe(false)
    expect(cancelledBar!.attributes('title')).toContain('Cancelled')
    expect(wrapper.text()).not.toContain('failed')
  })

  it('excludes an in-progress run from the duration range so it cannot pull the low end to 0s', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, durationSecs: 600, status: 'success' }),
          // A running backup carries durationSecs: 0 until it finishes.
          run({ id: 2, durationSecs: 0, status: 'started' }),
        ],
      },
    })
    expect(wrapper.text()).toContain('2 runs · 10m 0s')
    expect(wrapper.text()).not.toContain('0s-')
  })

  it('reports a plain run count with no range when nothing has completed yet', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [run({ id: 1, durationSecs: 0, status: 'started' })],
      },
    })
    expect(wrapper.text()).toContain('1 run')
    expect(wrapper.text()).not.toContain('·')
  })
})
