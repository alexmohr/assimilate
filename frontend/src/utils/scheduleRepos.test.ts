// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { failingRepoCount, scheduleRepoRuns } from './scheduleRepos'
import type { ReportRow } from '../types/report'
import type { ScheduleRepoOption } from '../types/schedule'

const PRIMARY: ScheduleRepoOption = { id: 20, name: 'server-daily', required: true }
const OFFSITE: ScheduleRepoOption = { id: 21, name: 'offsite-weekly', required: false }

function report(overrides: Partial<ReportRow>): ReportRow {
  return {
    id: 1,
    agent_id: 10,
    repo_id: PRIMARY.id,
    repo_name: PRIMARY.name,
    status: 'success',
    started_at: '2026-08-18T02:00:00Z',
    finished_at: '2026-08-18T02:06:41Z',
    original_size: 2_100_000_000,
    duration_secs: 401,
    archive_name: 'web-server-01-2026-08-18',
    error_message: null,
    warnings: [],
    ...overrides,
  } as unknown as ReportRow
}

describe('scheduleRepoRuns', () => {
  it('splits a run that fanned out into one entry per repository', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY, OFFSITE],
      [report({ id: 1 }), report({ id: 2, repo_id: OFFSITE.id, status: 'failed' })],
    )

    expect(runs.map((r) => r.repo.name)).toEqual(['server-daily', 'offsite-weekly'])
    expect(runs[0].status).toBe('success')
    expect(runs[1].status).toBe('failed')
  })

  it('keeps the targets in write order, not in the order runs arrived', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY, OFFSITE],
      [report({ id: 1, repo_id: OFFSITE.id }), report({ id: 2 })],
    )
    expect(runs.map((r) => r.repo.id)).toEqual([PRIMARY.id, OFFSITE.id])
  })

  it('reports a target that has never run as having no outcome', () => {
    const runs = scheduleRepoRuns([PRIMARY, OFFSITE], [report({ id: 1 })])
    expect(runs[1].last).toBeNull()
    expect(runs[1].status).toBeNull()
    expect(runs[1].reports).toEqual([])
  })

  it('takes the newest finished run as the outcome, whatever order it was given', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY],
      [
        report({ id: 1, finished_at: '2026-08-10T02:00:00Z', status: 'failed' }),
        report({ id: 2, finished_at: '2026-08-18T02:00:00Z', status: 'success' }),
        report({ id: 3, finished_at: '2026-08-14T02:00:00Z', status: 'warning' }),
      ],
    )
    expect(runs[0].last?.id).toBe(2)
    expect(runs[0].status).toBe('success')
    expect(runs[0].reports.map((r) => r.id)).toEqual([2, 3, 1])
  })

  // A run still going has no outcome to draw, and counting it would make the
  // newest cell of a strip flicker between states while a backup is running.
  it('leaves out runs that have not settled', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY],
      [report({ id: 1, status: 'started' }), report({ id: 2, status: 'pending' })],
    )
    expect(runs[0].reports).toEqual([])
    expect(runs[0].status).toBeNull()
  })

  it('keeps a cancelled run, which did settle', () => {
    const runs = scheduleRepoRuns([PRIMARY], [report({ id: 1, status: 'cancelled' })])
    expect(runs[0].status).toBe('cancelled')
  })

  // A repository removed from the schedule leaves its runs behind. Listing
  // them under a heading the schedule no longer writes to reports a failure
  // nobody can act on.
  it('drops runs against a repository that is no longer a target', () => {
    const runs = scheduleRepoRuns([PRIMARY], [report({ id: 1 }), report({ id: 2, repo_id: 99 })])
    expect(runs).toHaveLength(1)
    expect(runs[0].reports.map((r) => r.id)).toEqual([1])
  })

  it('has nothing to group for a schedule with no targets', () => {
    expect(scheduleRepoRuns([], [report({})])).toEqual([])
  })

  // A repository entry answers "is this copy current", which is a question
  // about the repository, so every host writing into it counts and the newest
  // run wins whoever produced it. The consequence, pinned here so it is a
  // decision rather than an accident: one host's failure sits behind another
  // host's later success. `failingRepoCount` is the per-host reading.
  it('takes the newest run of any host as the repository outcome', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY],
      [
        report({ id: 1, agent_id: 10, finished_at: '2026-08-18T02:00:00Z', status: 'failed' }),
        report({ id: 2, agent_id: 11, finished_at: '2026-08-18T03:00:00Z', status: 'success' }),
      ],
    )

    expect(runs[0].status).toBe('success')
    expect(runs[0].reports.map((r) => r.agent_id)).toEqual([11, 10])
    // The failure is not lost, only not what the repository row leads with.
    expect(failingRepoCount(runs, 10)).toBe(1)
    expect(failingRepoCount(runs, 11)).toBe(0)
  })
})

describe('failingRepoCount', () => {
  it('counts the repositories whose last run for this host failed', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY, OFFSITE],
      [report({ id: 1 }), report({ id: 2, repo_id: OFFSITE.id, status: 'failed' })],
    )
    expect(failingRepoCount(runs, 10)).toBe(1)
  })

  // A target row speaks for one machine; another host's failed copy is not
  // that row's problem.
  it('ignores another host runs', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY, OFFSITE],
      [report({ id: 1 }), report({ id: 2, repo_id: OFFSITE.id, agent_id: 11, status: 'failed' })],
    )
    expect(failingRepoCount(runs, 10)).toBe(0)
  })

  it('reads only the newest run, so a recovered repository stops counting', () => {
    const runs = scheduleRepoRuns(
      [PRIMARY],
      [
        report({ id: 1, finished_at: '2026-08-10T02:00:00Z', status: 'failed' }),
        report({ id: 2, finished_at: '2026-08-18T02:00:00Z', status: 'success' }),
      ],
    )
    expect(failingRepoCount(runs, 10)).toBe(0)
  })

  it('counts a repository this host has never run against as neither', () => {
    const runs = scheduleRepoRuns([PRIMARY, OFFSITE], [report({ id: 1 })])
    expect(failingRepoCount(runs, 10)).toBe(0)
  })
})
