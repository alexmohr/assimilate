// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleLogsTab from './ScheduleLogsTab.vue'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

const AGENTS = new Map<number, AgentRow>([
  [10, { id: 10, hostname: 'web-01', display_name: 'Web 01' } as unknown as AgentRow],
  [11, { id: 11, hostname: 'db-01', display_name: null } as unknown as AgentRow],
])

function report(overrides: Partial<ReportRow>): ReportRow {
  return {
    id: 1,
    status: 'success',
    started_at: '2026-01-01T02:00:00Z',
    finished_at: '2026-01-01T02:05:00Z',
    duration_secs: 300,
    original_size: 1024,
    agent_id: 10,
    error_message: null,
    ...overrides,
  } as unknown as ReportRow
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleLogsTab, {
    props: {
      reports: [report({})],
      loading: false,
      error: null,
      agents: AGENTS,
      ...props,
    },
  })
}

describe('ScheduleLogsTab', () => {
  it('shows a spinner while loading', () => {
    const wrapper = mount({ loading: true })
    expect(wrapper.find('.loading-row').exists()).toBe(true)
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('shows the error instead of the table', () => {
    const wrapper = mount({ error: 'could not reach the server' })
    expect(wrapper.find('.error-banner').text()).toBe('could not reach the server')
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('says so when the schedule has never run', () => {
    const wrapper = mount({ reports: [] })
    expect(wrapper.find('.state-msg').text()).toContain('No backup reports found')
  })

  it('prefers the display name and falls back to the hostname', () => {
    const wrapper = mount({
      reports: [report({ id: 1, agent_id: 10 }), report({ id: 2, agent_id: 11 })],
    })
    const hosts = wrapper.findAll('.cell-host').map((c) => c.text())
    expect(hosts).toEqual(['Web 01', 'db-01'])
  })

  it('names an unknown agent by id rather than blank', () => {
    const wrapper = mount({ reports: [report({ agent_id: 99 })] })
    expect(wrapper.find('.cell-host').text()).toBe('#99')
  })

  it('renders the run status through the shared badge vocabulary', () => {
    expect(mount().find('.badge').classes()).toContain('badge--success')
    expect(
      mount({ reports: [report({ status: 'failed' })] })
        .find('.badge')
        .classes(),
    ).toContain('badge--danger')
  })

  it('truncates a long error but keeps the whole text in the tooltip', () => {
    const long = 'x'.repeat(200)
    const wrapper = mount({ reports: [report({ error_message: long })] })
    const snippet = wrapper.find('.error-snippet')
    expect(snippet.text()).toHaveLength(81)
    expect(snippet.attributes('title')).toBe(long)
  })

  it('leaves a dash where there is no error', () => {
    expect(mount().find('.no-error').exists()).toBe(true)
  })
})
