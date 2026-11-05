// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  DEFAULT_RUN_ESTIMATE_SECS,
  MIN_RUN_ESTIMATE_SECS,
  estimateRunSecs,
  findScheduleOverlaps,
  overlappingAgents,
  type OverlapSchedule,
} from './scheduleOverlap'

const T0 = Date.UTC(2026, 10, 4, 2, 0)
const MIN = 60_000

function schedule(overrides: Partial<OverlapSchedule> & { id: number }): OverlapSchedule {
  return {
    agents: ['web-01'],
    repoIds: [20],
    nextStartMs: T0,
    recentDurationsSecs: [],
    ...overrides,
  }
}

describe('estimateRunSecs', () => {
  it('takes the median of an odd number of runs', () => {
    expect(estimateRunSecs([600, 120, 900])).toEqual({ secs: 600, fromHistory: true })
  })

  it('averages the two middle runs of an even number', () => {
    expect(estimateRunSecs([100, 200, 400, 10_000])).toEqual({ secs: 300, fromHistory: true })
  })

  it('assumes the default when there is no history', () => {
    expect(estimateRunSecs([])).toEqual({ secs: DEFAULT_RUN_ESTIMATE_SECS, fromHistory: false })
  })

  it('ignores durations that are not real numbers', () => {
    expect(estimateRunSecs([Number.NaN, -5, Number.POSITIVE_INFINITY])).toEqual({
      secs: DEFAULT_RUN_ESTIMATE_SECS,
      fromHistory: false,
    })
  })

  it('never estimates less than the minimum', () => {
    expect(estimateRunSecs([0, 0, 1])).toEqual({ secs: MIN_RUN_ESTIMATE_SECS, fromHistory: true })
  })
})

describe('findScheduleOverlaps', () => {
  it('flags two agents writing to the same repository at overlapping times', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'], recentDurationsSecs: [40 * 60] }),
      schedule({ id: 2, agents: ['db-01'], nextStartMs: T0 + 10 * MIN }),
    ])

    expect(overlaps).toHaveLength(1)
    expect(overlaps[0].repoId).toBe(20)
    expect(overlaps[0].first).toMatchObject({
      scheduleId: 1,
      agent: 'web-01',
      startMs: T0,
      endMs: T0 + 40 * MIN,
      estimate: { secs: 40 * 60, fromHistory: true },
    })
    expect(overlaps[0].second).toMatchObject({
      scheduleId: 2,
      agent: 'db-01',
      startMs: T0 + 10 * MIN,
      estimate: { fromHistory: false },
    })
  })

  it('orders each pair by start time, whatever order the schedules come in', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'], nextStartMs: T0 + 2 * MIN }),
      schedule({ id: 2, agents: ['db-01'], nextStartMs: T0 }),
    ])

    expect(overlaps[0].first.scheduleId).toBe(2)
    expect(overlaps[0].second.scheduleId).toBe(1)
  })

  it('does not flag runs into different repositories', () => {
    expect(
      findScheduleOverlaps([
        schedule({ id: 1, agents: ['web-01'], repoIds: [20] }),
        schedule({ id: 2, agents: ['db-01'], repoIds: [21] }),
      ]),
    ).toEqual([])
  })

  it('does not flag two schedules on the same agent', () => {
    expect(
      findScheduleOverlaps([
        schedule({ id: 1, agents: ['web-01'] }),
        schedule({ id: 2, agents: ['web-01'] }),
      ]),
    ).toEqual([])
  })

  it('flags the other agent of a schedule that shares one agent but runs on two', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01', 'db-01'] }),
      schedule({ id: 2, agents: ['web-01'], nextStartMs: T0 + MIN }),
    ])

    expect(overlaps).toHaveLength(1)
    expect(overlaps[0].first.agent).toBe('db-01')
    expect(overlaps[0].second.agent).toBe('web-01')
  })

  it('does not flag a run that starts exactly when the other is expected to end', () => {
    expect(
      findScheduleOverlaps([
        schedule({ id: 1, agents: ['web-01'], recentDurationsSecs: [10 * 60] }),
        schedule({ id: 2, agents: ['db-01'], nextStartMs: T0 + 10 * MIN }),
      ]),
    ).toEqual([])
  })

  it('flags a run that starts a second before the other is expected to end', () => {
    expect(
      findScheduleOverlaps([
        schedule({ id: 1, agents: ['web-01'], recentDurationsSecs: [10 * 60] }),
        schedule({ id: 2, agents: ['db-01'], nextStartMs: T0 + 10 * MIN - 1000 }),
      ]),
    ).toHaveLength(1)
  })

  it('assumes the default run length for a schedule with no history', () => {
    const justInside = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'] }),
      schedule({
        id: 2,
        agents: ['db-01'],
        nextStartMs: T0 + DEFAULT_RUN_ESTIMATE_SECS * 1000 - 1,
      }),
    ])
    const justOutside = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'] }),
      schedule({ id: 2, agents: ['db-01'], nextStartMs: T0 + DEFAULT_RUN_ESTIMATE_SECS * 1000 }),
    ])

    expect(justInside).toHaveLength(1)
    expect(justInside[0].first.estimate.fromHistory).toBe(false)
    expect(justOutside).toEqual([])
  })

  it('skips a schedule with no next run', () => {
    expect(
      findScheduleOverlaps([
        schedule({ id: 1, agents: ['web-01'] }),
        schedule({ id: 2, agents: ['db-01'], nextStartMs: null }),
      ]),
    ).toEqual([])
  })

  it('checks every repository of a multi-repository schedule', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'], repoIds: [20, 22] }),
      schedule({ id: 2, agents: ['db-01'], repoIds: [21, 22] }),
    ])

    expect(overlaps.map((o) => o.repoId)).toEqual([22])
  })

  it('reports each shared repository of one pair separately', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'], repoIds: [20, 21, 20] }),
      schedule({ id: 2, agents: ['db-01'], repoIds: [21, 20] }),
    ])

    expect(overlaps.map((o) => o.repoId)).toEqual([20, 21])
  })

  it('reports every overlapping pair among more than two schedules', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'], recentDurationsSecs: [60 * 60] }),
      schedule({ id: 2, agents: ['db-01'], nextStartMs: T0 + 10 * MIN }),
      schedule({ id: 3, agents: ['media-01'], nextStartMs: T0 + 12 * MIN }),
      // Same agent as schedule 1: overlaps 2 and 3, never 1.
      schedule({ id: 4, agents: ['web-01'], nextStartMs: T0 + 13 * MIN }),
      // Long after everything else has finished.
      schedule({ id: 5, agents: ['nas-01'], nextStartMs: T0 + 3 * 60 * MIN }),
    ])

    expect(overlaps.map((o) => [o.first.scheduleId, o.second.scheduleId])).toEqual([
      [1, 2],
      [1, 3],
      [2, 3],
      [2, 4],
      [3, 4],
    ])
  })
})

describe('overlappingAgents', () => {
  it('names, for each schedule, the agents it overlaps with once each', () => {
    const overlaps = findScheduleOverlaps([
      schedule({ id: 1, agents: ['web-01'], repoIds: [20, 21], recentDurationsSecs: [3600] }),
      schedule({ id: 2, agents: ['db-01'], repoIds: [20, 21], nextStartMs: T0 + MIN }),
      schedule({ id: 3, agents: ['media-01'], nextStartMs: T0 + 2 * MIN }),
    ])

    const byId = overlappingAgents(overlaps)
    expect(byId.get(1)).toEqual(['db-01', 'media-01'])
    expect(byId.get(2)).toEqual(['web-01', 'media-01'])
    expect(byId.get(3)).toEqual(['web-01', 'db-01'])
  })

  it('is empty without overlaps', () => {
    expect(overlappingAgents([]).size).toBe(0)
  })
})
