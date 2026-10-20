// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { retryableReportIds } from './reportRetry'
import type { ReportRow } from '../types/report'

function report(id: number, status: string, overrides: Partial<ReportRow> = {}): ReportRow {
  return { id, status, agent_id: 1, repo_id: 10, schedule_id: 5, ...overrides } as ReportRow
}

describe('retryableReportIds', () => {
  it('offers the newest failed run of each copy', () => {
    const reports = [
      report(1, 'failed', { repo_id: 10 }),
      report(2, 'failed', { repo_id: 11 }),
      report(3, 'success', { repo_id: 12 }),
    ]
    expect(retryableReportIds(reports)).toEqual(new Set([1, 2]))
  })

  it('does not offer a failure a later run of the same copy has replaced', () => {
    expect(retryableReportIds([report(1, 'failed'), report(2, 'success')])).toEqual(new Set())
  })

  it('stops offering it once a retry is pending', () => {
    // The pending row the server inserts for the retry is newer, so the
    // button goes away instead of inviting a second press.
    expect(retryableReportIds([report(1, 'failed'), report(2, 'pending')])).toEqual(new Set())
  })

  it('judges each copy on its own, regardless of the order rows arrive in', () => {
    const reports = [
      report(4, 'failed', { agent_id: 2 }),
      report(3, 'success', { agent_id: 1 }),
      report(1, 'failed', { agent_id: 1 }),
      report(2, 'failed', { agent_id: 2 }),
    ]
    expect(retryableReportIds(reports)).toEqual(new Set([4]))
  })

  it('treats the same host and repository under another schedule as another copy', () => {
    const reports = [
      report(1, 'failed', { schedule_id: 5 }),
      report(2, 'success', { schedule_id: 6 }),
    ]
    expect(retryableReportIds(reports)).toEqual(new Set([1]))
  })

  it('offers nothing for a run with no schedule, or one that only warned or was skipped', () => {
    const reports = [
      report(1, 'failed', { schedule_id: null }),
      report(2, 'warning', { repo_id: 11 }),
      report(3, 'skipped', { repo_id: 12 }),
      report(4, 'cancelled', { repo_id: 13 }),
    ]
    expect(retryableReportIds(reports)).toEqual(new Set())
  })
})
