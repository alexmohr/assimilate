// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { cronToHuman, classifyCron } from './cron'

describe('cronToHuman', () => {
  it('describes an hourly cadence', () => {
    expect(cronToHuman('0 */1 * * *')).toBe('Every hour')
    expect(cronToHuman('0 */6 * * *')).toBe('Every 6 hours')
  })

  it('describes an hourly cadence at a minute past the hour', () => {
    expect(cronToHuman('30 */6 * * *')).toBe('Every 6 hours at :30')
    expect(cronToHuman('5 * * * *')).toBe('Every hour at :05')
    expect(cronToHuman('* */6 * * *')).toBe('')
  })

  it('describes a daily cadence', () => {
    expect(cronToHuman('30 4 * * *')).toBe('Daily at 04:30')
  })

  it('describes a weekly cadence on one or more weekdays', () => {
    expect(cronToHuman('0 1 * * 2')).toBe('Tue at 01:00')
    expect(cronToHuman('0 1 * * 1,3,5')).toBe('Mon, Wed, Fri at 01:00')
  })

  it('names a weekday range instead of reading only its first day', () => {
    expect(cronToHuman('0 9 * * 1-5')).toBe('Mon–Fri at 09:00')
    expect(cronToHuman('0 9 * * 0,6')).toBe('Sun, Sat at 09:00')
    expect(cronToHuman('0 9 * * 1-3,5')).toBe('Mon–Wed, Fri at 09:00')
    // 7 is Sunday too.
    expect(cronToHuman('0 9 * * 5-7')).toBe('Fri–Sun at 09:00')
  })

  it('shows a weekday part it cannot read as written', () => {
    expect(cronToHuman('0 9 * * 1-9')).toBe('1-9 at 09:00')
    expect(cronToHuman('0 9 * * */2')).toBe('*/2 at 09:00')
    expect(cronToHuman('0 9 * * MON-FRI')).toBe('MON-FRI at 09:00')
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

describe('cronToHuman with a minute or hour that is not one value', () => {
  // `parseInt` reads `0,30` as `0`: these used to be described as running
  // once where they run several times, so they now get no description.
  it.each([
    ['0,30 */6 * * *', 'a list of minutes on an hourly cadence'],
    ['*/15 * * * *', 'a step of minutes'],
    ['0-5 * * * *', 'a range of minutes'],
    ['0 9,17 * * *', 'a list of hours'],
    ['0 9-17 * * 1-5', 'a range of hours'],
    ['0 */2 1 * *', 'a step of hours on a monthly cadence'],
    ['60 * * * *', 'a minute past 59'],
    ['0 24 * * *', 'an hour past 23'],
  ])('%s (%s)', (expr) => {
    expect(classifyCron(expr)).toBeNull()
    expect(cronToHuman(expr)).toBe('')
  })

  it('still reads one fixed minute and hour', () => {
    expect(cronToHuman('30 */6 * * *')).toBe('Every 6 hours at :30')
    expect(classifyCron('59 23 * * *')).toEqual({ kind: 'daily', hourNum: 23, minNum: 59 })
  })
})

describe('classifyCron', () => {
  it('returns null for anything cronToHuman cannot describe', () => {
    expect(classifyCron('not a cron')).toBeNull()
    expect(classifyCron('0 3 15 * 2')).toBeNull()
  })
})
