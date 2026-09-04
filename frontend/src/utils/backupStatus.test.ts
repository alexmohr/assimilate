// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { filterSettledReports, normalizeBackupStatus, reportMessageLabel } from './backupStatus'

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
