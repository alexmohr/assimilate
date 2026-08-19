// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { cronIntervalSecs } from './cadence'

describe('cronIntervalSecs', () => {
  it('computes hourly intervals', () => {
    expect(cronIntervalSecs('0 */8 * * *')).toBe(8 * 3600)
    expect(cronIntervalSecs('0 */1 * * *')).toBe(3600)
  })

  it('computes a daily interval', () => {
    expect(cronIntervalSecs('30 4 * * *')).toBe(24 * 3600)
  })

  it('computes a weekly interval for a single weekday', () => {
    expect(cronIntervalSecs('0 1 * * 2')).toBe(7 * 24 * 3600)
  })

  it('divides the week evenly across multiple weekdays', () => {
    expect(cronIntervalSecs('0 1 * * 1,3,5')).toBe(Math.round((7 * 24 * 3600) / 3))
  })

  it('approximates a monthly interval at 30 days', () => {
    expect(cronIntervalSecs('0 3 15 * *')).toBe(30 * 24 * 3600)
  })

  it('returns null for a malformed expression', () => {
    expect(cronIntervalSecs('not a cron')).toBeNull()
    expect(cronIntervalSecs('0 x * * *')).toBeNull()
  })

  it('returns null when both day-of-month and day-of-week are fixed, since there is no single interval', () => {
    expect(cronIntervalSecs('0 3 15 * 2')).toBeNull()
  })
})
