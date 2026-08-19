// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { cronToHuman, classifyCron } from './cron'

describe('cronToHuman', () => {
  it('describes an hourly cadence', () => {
    expect(cronToHuman('0 */1 * * *')).toBe('Every hour')
    expect(cronToHuman('0 */6 * * *')).toBe('Every 6 hours')
  })

  it('describes a daily cadence', () => {
    expect(cronToHuman('30 4 * * *')).toBe('Daily at 04:30')
  })

  it('describes a weekly cadence on one or more weekdays', () => {
    expect(cronToHuman('0 1 * * 2')).toBe('Tue at 01:00')
    expect(cronToHuman('0 1 * * 1,3,5')).toBe('Mon, Wed, Fri at 01:00')
  })

  it('falls back to the raw token for an out-of-range weekday number', () => {
    expect(cronToHuman('0 1 * * 9')).toBe('9 at 01:00')
  })

  it('describes a monthly cadence', () => {
    expect(cronToHuman('0 3 15 * *')).toBe('Monthly on day 15 at 03:00')
  })

  it('returns an empty string for a malformed or ambiguous expression', () => {
    expect(cronToHuman('not a cron')).toBe('')
    expect(cronToHuman('0 x * * *')).toBe('')
    // Both day-of-month and day-of-week fixed - no single supported shape.
    expect(cronToHuman('0 3 15 * 2')).toBe('')
  })
})

describe('classifyCron', () => {
  it('returns null for anything cronToHuman cannot describe', () => {
    expect(classifyCron('not a cron')).toBeNull()
    expect(classifyCron('0 3 15 * 2')).toBeNull()
  })
})
