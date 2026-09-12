// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import AgentScheduleRow from './AgentScheduleRow.vue'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'

function schedule(over: Record<string, unknown> = {}): ScheduleRow {
  return {
    id: 100,
    repo_id: 10,
    name: 'Dragon VMs',
    target_hostnames: ['dragon'],
    cron_expression: '0 20 1 * *',
    enabled: true,
    next_run_at: '2026-10-01T20:00:00Z',
    last_run_at: null,
    ...over,
  } as unknown as ScheduleRow
}

function health(over: Record<string, unknown> = {}): ScheduleHealthEntry {
  return {
    repo_id: 10,
    schedule_id: 100,
    hostname: 'dragon',
    target_name: 'Hetzner Global',
    last_status: 'failed',
    last_backup_at: '2026-06-01T02:00:00Z',
    last_backup_status: 'failed',
    is_overdue: false,
    last_error_message: null,
    cron_expression: '0 20 1 * *',
    schedule_enabled: true,
    consecutive_missed_backups: 0,
    missed_backup_threshold: 3,
    ...over,
  } as ScheduleHealthEntry
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentScheduleRow, {
    props: {
      schedule: schedule(),
      repoName: 'Hetzner Global',
      health: [health()],
      ...props,
    },
  })
}

describe('AgentScheduleRow', () => {
  it('reads "last run" off completed-backup evidence, not schedule.last_run_at', () => {
    // Reproduces the reported bug: a schedule whose trigger dispatch never
    // reached the agent (so `last_run_at` stayed null, per the fixture above)
    // but that nonetheless has a settled backup_reports row - e.g. abandoned
    // as failed once the agent reconnected. The row must not say "never run"
    // while the Recent backups list right below it shows this exact run.
    const wrapper = mount()
    expect(wrapper.find('.agent-row-stats').text()).toMatch(/last \d+[dhm] ago/)
    expect(wrapper.text()).not.toContain('never run')
  })

  it('says never run only when no health entry has a completed backup', () => {
    const wrapper = mount({ health: [health({ last_backup_at: null, last_status: null })] })
    expect(wrapper.text()).toContain('never run')
  })

  it('says never run when there is no health data at all', () => {
    const wrapper = mount({ health: [] })
    expect(wrapper.text()).toContain('never run')
  })

  it('picks the most recent completed backup across several health entries', () => {
    const recent = new Date(Date.now() - 60 * 60 * 1000).toISOString()
    const wrapper = mount({
      health: [
        health({ last_backup_at: '2026-05-01T00:00:00Z' }),
        health({ last_backup_at: recent }),
        health({ last_backup_at: '2026-04-01T00:00:00Z' }),
      ],
    })
    expect(wrapper.find('.agent-row-stats').text()).toContain('last 1h ago')
  })

  it('shows a humanized cadence, falling back to the raw expression', () => {
    expect(mount().find('.agent-row-when').text()).toBe('Monthly on day 1 at 20:00')
    // '?' isn't a shape classifyCron() understands - proves the fallback path.
    expect(
      mount({ schedule: schedule({ cron_expression: '? ? ? ? ?' }) })
        .find('.agent-row-when')
        .text(),
    ).toBe('? ? ? ? ?')
  })
})
