// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import AgentRunStrip from './AgentRunStrip.vue'
import type { ReportRow } from '../types/report'

/**
 * Reports as the API returns them: newest first. `finished_at` walks backwards
 * one hour at a time from a fixed instant so the fixtures are deterministic.
 */
function reports(statuses: readonly string[]): ReportRow[] {
  const base = Date.parse('2026-06-01T12:00:00Z')
  return statuses.map(
    (status, i) =>
      ({
        id: i + 1,
        repo_name: 'server-daily',
        status,
        finished_at: new Date(base - i * 3_600_000).toISOString(),
      }) as unknown as ReportRow,
  )
}

function mount(statuses: readonly string[], props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentRunStrip, { props: { reports: reports(statuses), ...props } })
}

function cellClasses(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('.run-cell').map((c) => c.classes().join(' '))
}

describe('AgentRunStrip', () => {
  it('draws one cell per run', () => {
    expect(mount(['success', 'success', 'failed']).findAll('.run-cell')).toHaveLength(3)
  })

  // Oldest on the left, so the strip reads as a timeline rather than needing
  // the reader to know the API's ordering.
  it('reverses the API order so the newest run is rightmost', () => {
    const classes = cellClasses(mount(['failed', 'success', 'success']))
    expect(classes[0]).toContain('run-cell--success')
    expect(classes[2]).toContain('run-cell--failed')
  })

  it('headlines the failure count', () => {
    expect(mount(['failed', 'success', 'failed']).text()).toContain('2 failed')
  })

  it('headlines warnings only when nothing failed', () => {
    expect(mount(['warning', 'success']).text()).toContain('1 with warnings')
    expect(mount(['failed', 'warning']).text()).toContain('1 failed')
  })

  it('says so when every run in the window is clean', () => {
    expect(mount(['success', 'success', 'success']).text()).toContain('All 3 clean')
  })

  it('handles an agent that has never run', () => {
    const wrapper = mount([])
    expect(wrapper.text()).toContain('No runs yet')
    expect(wrapper.findAll('.run-cell')).toHaveLength(0)
  })

  // A run in flight has no outcome yet; colouring it either way would make
  // the newest cell flicker while a backup is running.
  it('ignores pending and started runs', () => {
    const wrapper = mount(['started', 'pending', 'success', 'success'])
    expect(wrapper.findAll('.run-cell')).toHaveLength(2)
    expect(wrapper.text()).toContain('All 2 clean')
  })

  // The whole point of a count window over a calendar one: the number of
  // samples does not depend on how often the agent backs up.
  it('caps the window at the run limit regardless of cadence', () => {
    const wrapper = mount(Array<string>(50).fill('success'))
    expect(wrapper.findAll('.run-cell')).toHaveLength(20)
    expect(wrapper.text()).toContain('All 20 clean')
  })

  it('honours an explicit limit', () => {
    expect(
      mount(Array<string>(10).fill('success'), { limit: 4 }).findAll('.run-cell'),
    ).toHaveLength(4)
  })

  // One outage and a standing pattern produce the same count, so the count
  // alone cannot tell them apart - contiguity can.
  it('marks contiguous failures as a single incident', () => {
    const wrapper = mount(['success', 'failed', 'failed', 'failed', 'success'])
    expect(wrapper.text()).toContain('3 failed')
    expect(wrapper.find('.badge--danger').text()).toBe('Incident')
  })

  it('does not call scattered failures an incident', () => {
    const wrapper = mount(['success', 'failed', 'success', 'failed', 'success'])
    expect(wrapper.text()).toContain('2 failed')
    expect(wrapper.find('.badge--danger').exists()).toBe(false)
  })

  it('does not label a lone failure an incident', () => {
    expect(mount(['failed', 'success']).find('.badge--danger').exists()).toBe(false)
  })

  // A run count is cadence-independent by construction, which means the span
  // it covers varies wildly between agents - so the strip states it.
  it('names the time span the window actually covers', () => {
    expect(mount(['success', 'success', 'success']).text()).toMatch(/3 runs back to/)
  })

  it('describes itself for assistive tech', () => {
    const label = mount(['success', 'failed']).find('.run-strip-cells').attributes('aria-label')
    expect(label).toBe('Last 2 runs, oldest first: 1 failed')
  })
})
