// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleBackupsTab from './ScheduleBackupsTab.vue'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

const AGENTS = new Map<number, AgentRow>([
  [10, { id: 10, hostname: 'web-01', display_name: 'Web 01' } as unknown as AgentRow],
])

function report(overrides: Partial<ReportRow>): ReportRow {
  return {
    id: 1,
    status: 'success',
    archive_name: 'web-01-2026-01-01',
    started_at: '2026-01-01T02:00:00Z',
    original_size: 1024,
    deduplicated_size: 512,
    agent_id: 10,
    hostname: 'web-01',
    ...overrides,
  } as unknown as ReportRow
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleBackupsTab, {
    props: {
      reports: [report({})],
      loading: false,
      error: null,
      agents: AGENTS,
      repoId: 3,
      selected: null,
      ...props,
    },
  })
}

describe('ScheduleBackupsTab', () => {
  it('shows a spinner while loading', () => {
    expect(mount({ loading: true }).find('.reports-loading').exists()).toBe(true)
  })

  it('shows the error instead of the browser', () => {
    expect(mount({ error: 'boom' }).find('.error-banner').text()).toBe('boom')
  })

  it('lists only runs that produced an archive', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'kept' }),
        report({ id: 2, archive_name: null }),
        report({ id: 3, archive_name: 'failed-run', status: 'failed' }),
      ],
    })
    const names = wrapper.findAll('.cell-archive-name').map((c) => c.text())
    expect(names).toEqual(['kept'])
  })

  it('keeps warning runs, whose archive exists but is incomplete', () => {
    const wrapper = mount({
      reports: [report({ id: 2, archive_name: 'partial', status: 'warning' })],
    })
    expect(wrapper.findAll('.cell-archive-name').map((c) => c.text())).toEqual(['partial'])
  })

  it('orders the archives newest first', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'jan', started_at: '2026-01-01T00:00:00Z' }),
        report({ id: 2, archive_name: 'mar', started_at: '2026-03-01T00:00:00Z' }),
        report({ id: 3, archive_name: 'feb', started_at: '2026-02-01T00:00:00Z' }),
      ],
    })
    expect(wrapper.findAll('.cell-archive-name').map((c) => c.text())).toEqual([
      'mar',
      'feb',
      'jan',
    ])
  })

  it('says so when the schedule has produced no archives', () => {
    const wrapper = mount({ reports: [report({ archive_name: null })] })
    expect(wrapper.find('.empty-state').text()).toContain('No backup archives found')
  })

  it('reports the clicked archive up to the view, which owns the selection', async () => {
    const wrapper = mount()
    await wrapper.find('.archive-row').trigger('click')
    const updates = wrapper.emitted('update:selected')
    expect(updates).toHaveLength(1)
    expect((updates![0][0] as ReportRow).archive_name).toBe('web-01-2026-01-01')
  })

  it('marks the selected row', () => {
    const wrapper = mount({ selected: report({}) })
    expect(wrapper.find('.archive-row').classes()).toContain('selected')
  })

  it('leaves every row unmarked when nothing is selected', () => {
    expect(mount().find('.archive-row').classes()).not.toContain('selected')
  })
})
