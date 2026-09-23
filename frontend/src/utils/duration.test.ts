// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { type DurationUnit, humanizeMinutes, inUnit, naturalUnit, toMinutes } from './duration'

const ALL: readonly DurationUnit[] = ['minutes', 'hours', 'days', 'weeks']

describe('naturalUnit', () => {
  it('picks the coarsest unit the value divides into exactly', () => {
    expect(naturalUnit(45, ALL)).toBe('minutes')
    expect(naturalUnit(120, ALL)).toBe('hours')
    expect(naturalUnit(4320, ALL)).toBe('days')
    expect(naturalUnit(20_160, ALL)).toBe('weeks')
  })

  /** 90 minutes is 1.5 hours, and a field showing 1.5 invites an edit that has
   *  to be rounded back - so it stays in minutes. */
  it('stays fine-grained for a value that does not divide evenly', () => {
    expect(naturalUnit(90, ALL)).toBe('minutes')
    expect(naturalUnit(1_501, ALL)).toBe('minutes')
  })

  it('only offers units the caller allows', () => {
    expect(naturalUnit(4320, ['minutes', 'hours'])).toBe('hours')
    expect(naturalUnit(20_160, ['hours', 'days', 'weeks'])).toBe('weeks')
  })

  /** Zero divides into everything, so it would otherwise read as "0 weeks". */
  it('does not promote zero to the coarsest unit', () => {
    expect(naturalUnit(0, ALL)).toBe('minutes')
    expect(naturalUnit(0, ['hours', 'days', 'weeks'])).toBe('hours')
  })

  it('falls back to the finest allowed unit when nothing fits', () => {
    expect(naturalUnit(30, ['hours', 'days'])).toBe('hours')
  })
})

describe('conversion', () => {
  it('round-trips a value through its unit', () => {
    for (const [minutes, unit] of [
      [120, 'hours'],
      [4320, 'days'],
      [20_160, 'weeks'],
    ] as const) {
      expect(toMinutes(inUnit(minutes, unit), unit)).toBe(minutes)
    }
  })

  /** The column is an integer, so a typed 1.5 has to land on a whole minute. */
  it('rounds a fractional entry to whole minutes', () => {
    expect(toMinutes(1.5, 'hours')).toBe(90)
    expect(toMinutes(0.5, 'minutes')).toBe(1)
    expect(toMinutes(2.5, 'days')).toBe(3600)
  })
})

describe('humanizeMinutes', () => {
  it('reads as the largest whole unit it fits, singular where it should be', () => {
    expect(humanizeMinutes(1)).toBe('1 minute')
    expect(humanizeMinutes(45)).toBe('45 minutes')
    expect(humanizeMinutes(60)).toBe('1 hour')
    expect(humanizeMinutes(90)).toBe('90 minutes')
    expect(humanizeMinutes(1_440)).toBe('1 day')
    expect(humanizeMinutes(4_320)).toBe('3 days')
    expect(humanizeMinutes(10_080)).toBe('1 week')
  })

  it('reads zero in minutes rather than promoting it to the coarsest unit', () => {
    expect(humanizeMinutes(0)).toBe('0 minutes')
  })
})
