// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { scheduleRowHealth, type ScheduleRowHealthInput } from './scheduleRowHealth'
import type { ScheduleHealthEntry } from './scheduleHealth'

function entry(overrides: Partial<ScheduleHealthEntry> = {}): ScheduleHealthEntry {
  return {
    repo_id: 20,
    schedule_id: 1,
    hostname: 'web-01',
    target_name: 'server-daily',
    last_status: 'success',
    last_backup_at: '2026-11-04T02:00:00Z',
    last_backup_status: 'success',
    last_success_at: '2026-11-04T02:00:00Z',
    is_overdue: false,
    last_error_message: null,
    cron_expression: '0 2 * * *',
    schedule_enabled: true,
    consecutive_missed_backups: 0,
    missed_backup_threshold: 3,
    ...overrides,
  }
}

function input(overrides: Partial<ScheduleRowHealthInput> = {}): ScheduleRowHealthInput {
  return {
    enabled: true,
    disabledLabel: 'Enabled',
    autoDisabled: false,
    running: false,
    entries: [entry()],
    overdueTitle: 'web-01 — last backup: Nov 1',
    offlineAgents: [],
    catchUpPendingCount: 0,
    overlapAgents: [],
    ...overrides,
  }
}

describe('scheduleRowHealth', () => {
  it('reads Healthy for a schedule whose last run passed', () => {
    expect(scheduleRowHealth(input())).toEqual({
      label: 'Healthy',
      tone: 'success',
      title: '',
      problem: false,
    })
  })

  it('reads Not run yet when no target has run', () => {
    expect(scheduleRowHealth(input({ entries: [] })).label).toBe('Not run yet')
    expect(scheduleRowHealth(input({ entries: [entry({ last_status: null })] })).tone).toBe(
      'neutral',
    )
  })

  it('says Running while a run is in flight, whatever else is true', () => {
    const health = scheduleRowHealth(
      input({ running: true, entries: [entry({ is_overdue: true })] }),
    )
    expect(health).toMatchObject({ label: 'Running', tone: 'accent', problem: false })
  })

  it('shows the disabled label, as a problem only when the scheduler disabled it', () => {
    expect(scheduleRowHealth(input({ enabled: false, disabledLabel: 'Disabled' }))).toMatchObject({
      label: 'Disabled',
      tone: 'neutral',
      problem: false,
    })
    expect(
      scheduleRowHealth(
        input({ enabled: false, disabledLabel: 'Auto-disabled · error', autoDisabled: true }),
      ),
    ).toMatchObject({ label: 'Auto-disabled · error', tone: 'danger', problem: true })
  })

  it('combines overdue and missed runs into one warning chip', () => {
    const health = scheduleRowHealth(
      input({ entries: [entry({ is_overdue: true, consecutive_missed_backups: 1 })] }),
    )
    expect(health.label).toBe('Overdue · 1/3 missed')
    expect(health.tone).toBe('warning')
    expect(health.problem).toBe(true)
    expect(health.title).toContain('web-01 — last backup: Nov 1')
    expect(health.title).toContain('1 consecutive missed backup (agent or target unreachable)')
  })

  it('turns danger when a target failed, carrying its error in the title', () => {
    const health = scheduleRowHealth(
      input({
        entries: [
          entry({ is_overdue: true }),
          entry({ hostname: 'db-01', last_status: 'failed', last_error_message: 'Lock timeout' }),
        ],
      }),
    )
    expect(health.label).toBe('Overdue · Failed')
    expect(health.tone).toBe('danger')
    expect(health.title).toContain('Lock timeout')
  })

  it('names a warned run when none failed', () => {
    const health = scheduleRowHealth(
      input({ entries: [entry({ last_status: 'warning', last_error_message: 'file changed' })] }),
    )
    expect(health).toMatchObject({ label: 'Warning', tone: 'warning', title: 'file changed' })
  })

  it('leaves out a missed streak that already reached the threshold', () => {
    const health = scheduleRowHealth(
      input({ entries: [entry({ consecutive_missed_backups: 3, missed_backup_threshold: 3 })] }),
    )
    expect(health.label).toBe('Healthy')
  })

  it('names each agent the schedule overlaps with', () => {
    const health = scheduleRowHealth(input({ overlapAgents: ['db-01', 'media-01'] }))
    expect(health.label).toBe('Overlaps db-01 · Overlaps media-01')
    expect(health.tone).toBe('warning')
    expect(health.title).toContain('same repository as db-01')
  })

  it('flags an offline agent on an otherwise healthy schedule', () => {
    const health = scheduleRowHealth(input({ offlineAgents: ['web-01 (last seen Nov 2, 10:00)'] }))
    expect(health).toMatchObject({
      label: 'Agent offline',
      tone: 'warning',
      title: 'web-01 (last seen Nov 2, 10:00)',
      problem: true,
    })
  })

  it('says a catch-up is pending when nothing is wrong', () => {
    const health = scheduleRowHealth(input({ catchUpPendingCount: 1 }))
    expect(health).toMatchObject({ label: 'Catch-up pending', tone: 'info', problem: false })
    expect(health.title).toContain('One host missed a run')
  })

  it('lets a failure outrank an offline agent, keeping a pending catch-up beside it', () => {
    const health = scheduleRowHealth(
      input({
        entries: [entry({ last_status: 'failed' })],
        offlineAgents: ['web-01'],
        catchUpPendingCount: 2,
      }),
    )
    expect(health.label).toBe('Failed · Catch-up pending')
    expect(health.tone).toBe('danger')
  })

  it('names a pending catch-up rather than the offline agent it is waiting for', () => {
    const health = scheduleRowHealth(input({ offlineAgents: ['web-01'], catchUpPendingCount: 1 }))
    expect(health).toMatchObject({ label: 'Catch-up pending', tone: 'info' })
  })

  it('keeps a pending catch-up on an overlap chip', () => {
    const health = scheduleRowHealth(input({ overlapAgents: ['web-02'], catchUpPendingCount: 1 }))
    expect(health.label).toBe('Overlaps web-02 · Catch-up pending')
    expect(health.title).toContain('One host missed a run')
  })
})
