// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { filterSettledReports, normalizeBackupStatus } from './backupStatus'

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
