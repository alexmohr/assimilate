// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  byFinishedDesc,
  filterSettledReports,
  normalizeBackupStatus,
  reportMessageLabel,
} from './backupStatus'

describe('normalizeBackupStatus', () => {
  it.each([
    ['success', 'success'],
    ['WARNING', 'warning'],
    ['Started', 'started'],
    ['pending', 'pending'],
    ['Cancelled', 'cancelled'],
    ['error', 'failed'],
    ['anything-else', 'failed'],
  ])('normalizes %s to %s', (raw, expected) => {
    expect(normalizeBackupStatus(raw)).toBe(expected)
  })
})

describe('filterSettledReports', () => {
  it('drops pending and started reports, keeps everything else', () => {
    const reports = [
      { id: 1, status: 'success' },
      { id: 2, status: 'pending' },
      { id: 3, status: 'started' },
      { id: 4, status: 'failed' },
      { id: 5, status: 'warning' },
      { id: 6, status: 'cancelled' },
    ]

    expect(filterSettledReports(reports).map((r) => r.id)).toEqual([1, 4, 5, 6])
  })

  it('returns an empty array when everything is still running', () => {
    expect(filterSettledReports([{ status: 'pending' }, { status: 'started' }])).toEqual([])
  })
})

describe('reportMessageLabel', () => {
  it('names the error of a failed run', () => {
    expect(reportMessageLabel({ status: 'failed', error_message: 'Lock held', warnings: [] })).toBe(
      'View error',
    )
  })

  // A warned run carries the same text in `error_message` (the notification
  // path reads it there), and what gets rendered is the warnings - so that is
  // what the jump has to be named after.
  it('prefers warnings over the error message they were downgraded from', () => {
    expect(
      reportMessageLabel({
        status: 'warning',
        error_message: 'file changed while we read it',
        warnings: ['file changed while we read it'],
      }),
    ).toBe('View warnings')
  })

  it('has nothing to say about a clean run', () => {
    expect(reportMessageLabel({ status: 'success', error_message: null, warnings: [] })).toBeNull()
  })

  // A success that still carries a message says nothing went wrong; only the
  // warnings list can make a successful run worth reading.
  it('ignores an error message left on a successful run', () => {
    expect(
      reportMessageLabel({ status: 'success', error_message: 'stale', warnings: [] }),
    ).toBeNull()
  })

  // The wire type promises an array, but reports reach the UI without one.
  it('treats missing warnings as none', () => {
    expect(reportMessageLabel({ status: 'failed', error_message: 'Lock held' })).toBe('View error')
    expect(reportMessageLabel({ status: 'cancelled', error_message: null })).toBeNull()
  })
})

describe('byFinishedDesc', () => {
  it('puts the newest finished run first', () => {
    const rows = [
      { id: 1, finished_at: '2026-08-10T02:00:00Z' },
      { id: 2, finished_at: '2026-08-18T02:00:00Z' },
      { id: 3, finished_at: '2026-08-14T02:00:00Z' },
    ]
    expect([...rows].sort(byFinishedDesc).map((r) => r.id)).toEqual([2, 3, 1])
  })

  // `finished_at` has one-second resolution, so a retry landing in the same
  // second as the run it replaces compares equal. `Array.sort` is stable, so
  // without a tie-break the caller's arrival order would decide which of them
  // counts as the last outcome - and a stale failure could outrank a newer
  // success on a status screen.
  it('breaks a tie on the later id, whatever order it was given', () => {
    const older = { id: 7, finished_at: '2026-08-18T02:00:00Z' }
    const newer = { id: 8, finished_at: '2026-08-18T02:00:00Z' }

    expect([older, newer].sort(byFinishedDesc).map((r) => r.id)).toEqual([8, 7])
    expect([newer, older].sort(byFinishedDesc).map((r) => r.id)).toEqual([8, 7])
  })
})
