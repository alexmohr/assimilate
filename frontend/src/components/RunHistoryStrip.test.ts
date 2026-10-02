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

  it('draws one bar per run, split into an equal-height segment per agent', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, runId: 'r1', hostname: 'web-01', startedAt: '2026-06-01T02:00:00Z' }),
          run({ id: 2, runId: 'r1', hostname: 'db-01', startedAt: '2026-06-01T02:10:00Z' }),
          run({ id: 3, runId: 'r2', hostname: 'web-01', startedAt: '2026-06-02T02:00:00Z' }),
          run({ id: 4, runId: 'r2', hostname: 'db-01', startedAt: '2026-06-02T02:10:00Z' }),
        ],
      },
    })
    const bars = wrapper.findAll('.run-bar')
    expect(bars.map((b) => b.attributes('data-run-id'))).toEqual(['r1', 'r2'])
    const segments = bars[0]!.findAll('.run-bar-segment')
    // Oldest target first; the bar stacks them bottom-up with equal flex share.
    expect(segments.map((s) => s.attributes('data-entry-id'))).toEqual(['1', '2'])
    expect(segments[1]!.attributes('title')).toContain('db-01')
    expect(bars[0]!.attributes('title')).toContain('2 agents')
    // Targets run one after another, so a firing's duration is their sum.
    expect(wrapper.text()).toContain('2 runs · 20m 0s')
  })

  it('colors each agent segment by its own outcome and counts a run with any failure as failed', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, runId: 'r1', status: 'success' }),
          run({ id: 2, runId: 'r1', status: 'failed', startedAt: '2026-06-01T02:10:00Z' }),
        ],
      },
    })
    const bars = wrapper.findAll('.run-bar')
    expect(bars).toHaveLength(1)
    expect(bars[0]!.classes()).toContain('run-bar-danger')
    expect(bars[0]!.attributes('style')).toContain('height: 100%')
    expect(bars[0]!.find('.run-bar-segment-success').exists()).toBe(true)
    expect(bars[0]!.find('.run-bar-segment-danger').exists()).toBe(true)
    expect(wrapper.text()).toContain('1 run · 1 failed')
  })

  it('draws a pending multi-agent run as one bar instead of a dot per agent', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, startedAt: '2026-06-01T02:00:00Z', durationSecs: 600 }),
          run({
            id: 2,
            runId: 'r2',
            durationSecs: 0,
            status: 'pending',
            startedAt: '2026-06-02T02:00:00Z',
          }),
          run({
            id: 3,
            runId: 'r2',
            durationSecs: 0,
            status: 'pending',
            startedAt: '2026-06-02T02:00:00Z',
          }),
        ],
      },
    })
    const bars = wrapper.findAll('.run-bar')
    expect(bars).toHaveLength(2)
    const pending = bars[1]!
    expect(pending.classes()).toContain('run-bar-accent')
    expect(pending.findAll('.run-bar-segment-accent')).toHaveLength(2)
    // The floor grows with the segment count so each agent stays visible.
    expect(pending.attributes('style')).toContain('height: 30%')
  })

  it('applies maxBars to runs, not to individual agent reports', () => {
    const runs = Array.from({ length: 6 }, (_, i) =>
      [0, 1].map((target) =>
        run({
          id: `${i}-${target}`,
          runId: `run-${i}`,
          startedAt: `2026-06-0${i + 1}T02:0${target}:00Z`,
        }),
      ),
    ).flat()
    const wrapper = mount(RunHistoryStrip, { props: { runs, maxBars: 4 } })
    const bars = wrapper.findAll('.run-bar')
    expect(bars.map((b) => b.attributes('data-run-id'))).toEqual([
      'run-2',
      'run-3',
      'run-4',
      'run-5',
    ])
    expect(bars.every((b) => b.findAll('.run-bar-segment').length === 2)).toBe(true)
  })

  // A run_id is shared by every (agent, repository) target of a firing, so a
  // two-agent, two-repository schedule writes four reports per run. The
  // segments must say which repository each is, and the bar must not call
  // four targets "4 agents".
  it('names the repository of each segment on a multi-repository run', () => {
    const wrapper = mount(RunHistoryStrip, {
      props: {
        runs: [
          run({ id: 1, runId: 'r1', hostname: 'web-01', targetName: 'local' }),
          run({ id: 2, runId: 'r1', hostname: 'web-01', targetName: 'offsite' }),
          run({ id: 3, runId: 'r1', hostname: 'db-01', targetName: 'local' }),
          run({ id: 4, runId: 'r1', hostname: 'db-01', targetName: 'offsite' }),
        ],
      },
    })
    const bar = wrapper.find('.run-bar')
    expect(bar.findAll('.run-bar-segment')).toHaveLength(4)
    expect(bar.attributes('title')).toContain('2 agents, 4 targets')
    const titles = bar.findAll('.run-bar-segment').map((s) => s.attributes('title'))
    expect(titles.some((t) => t?.includes('web-01 → local'))).toBe(true)
    expect(titles.some((t) => t?.includes('web-01 → offsite'))).toBe(true)
  })
})
