// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleOverviewTab from './ScheduleOverviewTab.vue'
import type { ScheduleRow } from '../types/schedule'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

const SCHEDULE = {
  id: 1,
  name: 'Nightly production backup',
  schedule_type: 'backup',
  repo_id: 20,
  on_failure: 'continue',
  next_run_at: '2026-08-19T02:00:00Z',
  last_run_at: '2026-08-18T02:00:00Z',
  enabled: true,
} as unknown as ScheduleRow

const AGENT_LABELS: Record<number, string> = { 10: 'web-server-01', 11: 'db-server-01' }

function mount(overrides: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleOverviewTab, {
    props: {
      schedule: SCHEDULE,
      repoName: 'server-daily',
      cronSummary: 'Daily at 02:00',
      agentIds: [10, 11],
      agentLabel: (id: number) => AGENT_LABELS[id] ?? `#${id}`,
      healthForAgent: (): HealthSummaryResponse | null => null,
      connectivityNote: () => '',
      retryingAgentId: null,
      reports: [] as ReportRow[],
      agents: new Map<number, AgentRow>(),
      backupRunning: false,
      backupHostname: null,
      backupArchiveName: null,
      backupElapsedSecs: 0,
      estimatedRemainingSecs: null,
      archiveProgress: null,
      ...overrides,
    },
  })
}

describe('ScheduleOverviewTab', () => {
  it('shows the schedule info summary', () => {
    const wrapper = mount()
    const text = wrapper.text()
    expect(text).toContain('server-daily')
    expect(text).toContain('Continue')
    expect(text).toContain('Daily at 02:00')
  })

  it('shows Never for a null last run', () => {
    const wrapper = mount({ schedule: { ...SCHEDULE, last_run_at: null } })
    expect(wrapper.text()).toContain('Never')
  })

  it('shows the live progress card only while a backup is running', () => {
    expect(mount({ backupRunning: true }).find('.live-log-card').exists()).toBe(true)
    expect(mount({ backupRunning: false }).find('.live-log-card').exists()).toBe(false)
  })

  it('lists every target as a row', () => {
    const wrapper = mount()
    const rows = wrapper.findAll('.agent-row')
    expect(rows.some((r) => r.text().includes('web-server-01'))).toBe(true)
    expect(rows.some((r) => r.text().includes('db-server-01'))).toBe(true)
  })

  it('shows no attention block when nothing is overdue', () => {
    expect(mount().find('.attention').exists()).toBe(false)
  })

  it('flags an overdue target with a connectivity note and a Retry button', async () => {
    const wrapper = mount({
      healthForAgent: (id: number): HealthSummaryResponse | null =>
        id === 10
          ? ({
              is_overdue: true,
              last_backup_at: '2026-08-15T02:00:00Z',
              last_status: 'success',
            } as unknown as HealthSummaryResponse)
          : null,
      connectivityNote: (id: number) => (id === 10 ? 'Agent offline (last seen 3 days ago)' : ''),
    })

    expect(wrapper.find('.attention').exists()).toBe(true)
    expect(wrapper.text()).toContain('web-server-01 has not run since')
    expect(wrapper.text()).toContain('Agent offline (last seen 3 days ago)')

    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Retry')
    expect(retryButton).toBeTruthy()
    await retryButton!.trigger('click')
    expect(wrapper.emitted('retry')).toEqual([[10]])
  })

  it('shows an Overdue badge and a Retry button on the target row itself, not just in the attention banner', async () => {
    const wrapper = mount({
      healthForAgent: (id: number): HealthSummaryResponse | null =>
        id === 10
          ? ({ is_overdue: true, last_backup_at: null } as unknown as HealthSummaryResponse)
          : null,
    })

    const targetRow = wrapper.findAll('.agent-row').find((r) => r.text().includes('web-server-01'))
    expect(targetRow).toBeTruthy()
    expect(targetRow!.find('.badge--warning').text()).toBe('Overdue')

    const retryButton = targetRow!.findAll('button').find((b) => b.text() === 'Retry')
    expect(retryButton).toBeTruthy()
    await retryButton!.trigger('click')
    expect(wrapper.emitted('retry')).toEqual([[10]])
  })

  it('disables Retry for the agent currently retrying', () => {
    const wrapper = mount({
      healthForAgent: (): HealthSummaryResponse | null =>
        ({ is_overdue: true, last_backup_at: null }) as unknown as HealthSummaryResponse,
      retryingAgentId: 10,
    })

    const retryButtons = wrapper
      .findAll('button')
      .filter((b) => b.text().includes('Retry') || b.text() === '...')
    expect(retryButtons.some((b) => b.attributes('disabled') !== undefined)).toBe(true)
  })

  it('hides the backups preview when there are no settled reports', () => {
    expect(
      mount({ reports: [{ id: 1, status: 'started' }] as unknown as ReportRow[] }).text(),
    ).not.toContain('Recent backups')
  })

  it('previews recent backups and emits openBackups from the view-all link', async () => {
    const agents = new Map<number, AgentRow>([
      [10, { id: 10, hostname: 'web-server-01', display_name: null } as unknown as AgentRow],
    ])
    const wrapper = mount({
      agents,
      reports: [
        {
          id: 1,
          agent_id: 10,
          status: 'success',
          finished_at: '2026-08-18T02:06:41Z',
          original_size: 2_100_000_000,
          duration_secs: 401,
        },
        {
          id: 2,
          agent_id: 10,
          status: 'failed',
          finished_at: '2026-08-15T02:00:00Z',
          original_size: 0,
          duration_secs: 5,
        },
      ] as unknown as ReportRow[],
    })

    expect(wrapper.text()).toContain('Recent backups')
    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('failed')

    await wrapper.find('.section-link').trigger('click')
    expect(wrapper.emitted('openBackups')).toHaveLength(1)
  })
})
