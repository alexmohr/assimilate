// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * Minute-valued settings written and read in whichever unit they read
 * naturally in.
 *
 * Every catch-up field on a schedule is stored as a whole number of minutes,
 * because that is the finest thing any of them means. Nobody thinks in minutes
 * past the first hour, though: a nightly schedule's collision floor is "two
 * hours", and a weekly schedule's is "two days". Storing one unit and
 * displaying another is what these helpers are for.
 */

export const MINUTES_PER_HOUR = 60
export const MINUTES_PER_DAY = 1_440
export const MINUTES_PER_WEEK = 10_080

export type DurationUnit = 'minutes' | 'hours' | 'days' | 'weeks'

const MINUTES_IN: Record<DurationUnit, number> = {
  minutes: 1,
  hours: MINUTES_PER_HOUR,
  days: MINUTES_PER_DAY,
  weeks: MINUTES_PER_WEEK,
}

/** Largest first, so `naturalUnit` finds the coarsest exact fit. */
const COARSEST_FIRST: readonly DurationUnit[] = ['weeks', 'days', 'hours', 'minutes']

/**
 * The coarsest of `allowed` that `minutes` divides into exactly.
 *
 * 120 minutes reads as 2 hours, 4320 as 3 days, and 90 stays 90 minutes
 * because rendering it as 1.5 hours would invite a fractional edit that the
 * field then has to round back.
 *
 * `allowed` is finest first, and its first entry is the fallback for anything
 * that fits no unit cleanly - including zero, which divides into every unit
 * and would otherwise be displayed as "0 weeks".
 */
export function naturalUnit(minutes: number, allowed: readonly DurationUnit[]): DurationUnit {
  const found = COARSEST_FIRST.find(
    (unit) =>
      allowed.includes(unit) && minutes >= MINUTES_IN[unit] && minutes % MINUTES_IN[unit] === 0,
  )
  return found ?? allowed[0] ?? 'minutes'
}

/** How many whole `unit`s `minutes` is, unrounded. */
export function inUnit(minutes: number, unit: DurationUnit): number {
  return minutes / MINUTES_IN[unit]
}

/** `value` of `unit`, as the whole minutes the column stores. */
export function toMinutes(value: number, unit: DurationUnit): number {
  return Math.round(value * MINUTES_IN[unit])
}

/**
 * A minute count as prose - "2 hours", "3 days", "45 minutes".
 *
 * Singular where it should be: "1 day" rather than "1 days", which is the sort
 * of thing that makes a status line look machine-generated.
 */
export function humanizeMinutes(minutes: number): string {
  const unit = naturalUnit(minutes, COARSEST_FIRST)
  const value = inUnit(minutes, unit)
  return value === 1 ? `1 ${unit.slice(0, -1)}` : `${value} ${unit}`
}
