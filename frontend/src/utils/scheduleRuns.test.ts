// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  countOutcomes,
  groupScheduleRuns,
  isCompleted,
  reportToOpen,
  worstOutcome,
} from './scheduleRuns'
import type { ReportRow } from '../types/report'

function report(id: number, over: Record<string, unknown> = {}): ReportRow {
  return {
    id,
    status: 'success',
    finished_at: `2026-08-18T02:${String(id).padStart(2, '0')}:00Z`,
    duration_secs: 10,
    run_id: null,
    ...over,
  } as unknown as ReportRow
}

describe('scheduleRuns', () => {
  it('ranks failed over warning over skipped over cancelled over success', () => {
    expect(worstOutcome([report(1), report(2, { status: 'cancelled' })])).toBe('cancelled')
    expect(
      worstOutcome([report(1, { status: 'cancelled' }), report(2, { status: 'skipped' })]),
    ).toBe('skipped')
    expect(worstOutcome([report(1, { status: 'skipped' }), report(2, { status: 'warning' })])).toBe(
      'warning',
    )
    expect(worstOutcome([report(1, { status: 'warning' }), report(2, { status: 'failed' })])).toBe(
      'failed',
    )
    expect(worstOutcome([])).toBe('success')
  })

  it('groups reports by run, newest run first, reports in the order they finished', () => {
    const runs = groupScheduleRuns([
      report(4, { run_id: 'b' }),
      report(3, { run_id: 'b', status: 'failed' }),
      report(2),
      report(1, { run_id: 'a', status: 'started' }),
    ])
    expect(runs.map((r) => r.key)).toEqual(['b', 'report-2'])
    expect(runs[0].reports.map((r) => r.id)).toEqual([3, 4])
    expect(runs[0].status).toBe('failed')
    expect(runs[0].durationSecs).toBe(20)
    expect(runs[0].finishedAt).toBe('2026-08-18T02:04:00Z')
  })

  it('keeps only the newest runs', () => {
    const reports = Array.from({ length: 25 }, (_, i) => report(25 - i))
    const runs = groupScheduleRuns(reports)
    expect(runs).toHaveLength(20)
    expect(runs[0].key).toBe('report-25')
  })

  it('counts runs by outcome and opens the first report that did not succeed', () => {
    const runs = groupScheduleRuns([
      report(3, { run_id: 'b', status: 'skipped' }),
      report(2, { run_id: 'b' }),
      report(1, { status: 'warning' }),
    ])
    expect(countOutcomes(runs)).toEqual({
      success: 0,
      warning: 1,
      skipped: 1,
      cancelled: 0,
      failed: 0,
    })
    expect(runs.filter(isCompleted)).toHaveLength(1)
    expect(reportToOpen(runs[0])?.id).toBe(3)
    expect(reportToOpen(groupScheduleRuns([report(5)])[0])?.id).toBe(5)
  })
})
