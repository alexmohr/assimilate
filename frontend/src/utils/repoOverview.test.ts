// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { buildRepoOverview, type RepoOverviewInput } from './repoOverview'
import type { ReportRow } from '../types/report'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleHealthEntry } from './scheduleHealth'

function report(over: Record<string, unknown>): ReportRow {
  return {
    id: 1,
    repo_id: 10,
    status: 'success',
    finished_at: '2026-06-01T10:00:00Z',
    ...over,
  } as unknown as ReportRow
}

function schedule(over: Record<string, unknown>): ScheduleRow {
  return {
    id: 1,
    repo_id: 10,
    enabled: true,
    next_run_at: '2026-06-02T02:00:00Z',
    ...over,
  } as unknown as ScheduleRow
}

function build(over: Partial<RepoOverviewInput> = {}) {
  return buildRepoOverview({
    repos: [{ id: 10, name: 'daily' }],
    reports: [],
    schedules: [],
    health: [],
    runningRepoIds: [],
    ...over,
  })
}

describe('buildRepoOverview', () => {
  it('has no state for a repository that never ran', () => {
    expect(build()[0]).toMatchObject({ state: 'none', lastRun: null, lastGood: null })
  })

  it('takes the state from the newest settled run', () => {
    const rows = build({
      reports: [
        report({ id: 1, status: 'failed', finished_at: '2026-06-01T08:00:00Z' }),
        report({ id: 2, status: 'success', finished_at: '2026-06-01T09:00:00Z' }),
      ],
    })
    expect(rows[0].state).toBe('success')
  })

  it('ignores runs that have not finished', () => {
    const rows = build({ reports: [report({ id: 2, status: 'started' }), report({ id: 1 })] })
    expect(rows[0].lastRun?.id).toBe(1)
  })

  it("keeps one repository's runs out of another's", () => {
    const rows = buildRepoOverview({
      repos: [
        { id: 10, name: 'a' },
        { id: 11, name: 'b' },
      ],
      reports: [report({ id: 1, repo_id: 11, status: 'failed' })],
      schedules: [],
      health: [],
      runningRepoIds: [],
    })
    expect(rows.map((r) => r.state)).toEqual(['none', 'failed'])
  })

  it('finds the last good run behind a failure', () => {
    const rows = build({
      reports: [
        report({ id: 2, status: 'failed', finished_at: '2026-06-02T10:00:00Z' }),
        report({ id: 1, status: 'warning', finished_at: '2026-06-01T10:00:00Z' }),
      ],
    })
    expect(rows[0].lastGood?.id).toBe(1)
  })

  describe('precedence', () => {
    const failed = [report({ status: 'failed' })]
    const overdue = [{ repo_id: 10, is_overdue: true } as ScheduleHealthEntry]

    it('running beats everything', () => {
      expect(build({ reports: failed, runningRepoIds: [10] })[0].state).toBe('running')
    })

    it('failed beats overdue', () => {
      expect(build({ reports: failed, health: overdue })[0].state).toBe('failed')
    })

    it('overdue beats a clean last run', () => {
      expect(build({ reports: [report({})], health: overdue })[0].state).toBe('overdue')
    })

    it("ignores another repository's overdue schedule", () => {
      const other = [{ repo_id: 99, is_overdue: true } as ScheduleHealthEntry]
      expect(build({ reports: [report({})], health: other })[0].state).toBe('success')
    })

    it('reports warnings', () => {
      expect(build({ reports: [report({ status: 'warning' })] })[0].state).toBe('warning')
    })
  })

  describe('next run', () => {
    it('is the soonest enabled schedule for this repository', () => {
      const rows = build({
        schedules: [
          schedule({ id: 1, next_run_at: '2026-06-05T02:00:00Z' }),
          schedule({ id: 2, next_run_at: '2026-06-02T02:00:00Z' }),
          schedule({ id: 3, repo_id: 99, next_run_at: '2026-06-01T02:00:00Z' }),
        ],
      })
      expect(rows[0].nextRunAt).toBe('2026-06-02T02:00:00Z')
    })

    // The server still reports a next_run_at for a disabled schedule.
    it('skips disabled schedules', () => {
      expect(build({ schedules: [schedule({ enabled: false })] })[0].nextRunAt).toBeNull()
    })
  })
})
