// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { getConfiguredTimezone } from '../composables/useTimezone'

// Cron field syntax tokens (POSIX cron format), not app-owned domain state.
export const CRON_ANY = '*'
export const CRON_TOP_OF_HOUR = '0'

function cronTimeToDisplay(hourNum: number, minNum: number): string {
  const displayTz = getConfiguredTimezone()
  if (displayTz) {
    return `${hourNum.toString().padStart(2, '0')}:${minNum.toString().padStart(2, '0')}`
  }
  const now = new Date()
  const refDate = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate(), hourNum, minNum),
  )
  const parts = new Intl.DateTimeFormat('en-US', {
    hour: 'numeric',
    minute: 'numeric',
    hour12: false,
  }).formatToParts(refDate)
  const h = parseInt(parts.find((p) => p.type === 'hour')?.value ?? '0', 10)
  const m = parseInt(parts.find((p) => p.type === 'minute')?.value ?? '0', 10)
  return `${h.toString().padStart(2, '0')}:${m.toString().padStart(2, '0')}`
}

/**
 * The cron shapes this app understands, classified once so callers that need
 * different projections of the same expression don't each re-implement field
 * parsing and shape detection and risk drifting out of sync.
 */
export type CronShape =
  | { kind: 'hourly'; intervalHours: number; minNum: number }
  | { kind: 'daily'; hourNum: number; minNum: number }
  | { kind: 'weekly'; dow: string; hourNum: number; minNum: number }
  | { kind: 'monthly'; dom: string; hourNum: number; minNum: number }

/**
 * One fixed value of a cron field, or null for a list, range, step or
 * anything out of range: `parseInt` alone reads `0,30` as `0` and would
 * describe a schedule that runs twice an hour as running once.
 */
function fixedValue(field: string, max: number): number | null {
  if (!/^\d+$/.test(field)) return null
  const value = Number(field)
  return value <= max ? value : null
}

export function classifyCron(expr: string): CronShape | null {
  const parts = expr.trim().split(/\s+/)
  if (parts.length !== 5) return null

  const [min = '', hour = '', dom, , dow] = parts

  const minNum = fixedValue(min, 59)
  if (minNum === null) return null

  // `*` and `*/n` in the hour field, at any fixed minute: `30 */6 * * *` is
  // every six hours at half past, which used to get no description at all.
  const hourlyMatch = hour === CRON_ANY ? ['*', '1'] : hour.match(/^\*\/(\d+)$/)
  if (hourlyMatch && dom === CRON_ANY && dow === CRON_ANY) {
    return { kind: 'hourly', intervalHours: parseInt(hourlyMatch[1] ?? '1', 10), minNum }
  }

  const hourNum = fixedValue(hour, 23)
  if (hourNum === null) return null

  if (dom === CRON_ANY && dow === CRON_ANY) return { kind: 'daily', hourNum, minNum }
  if (dom === CRON_ANY && dow !== CRON_ANY) return { kind: 'weekly', dow, hourNum, minNum }
  if (dow === CRON_ANY && dom !== CRON_ANY) return { kind: 'monthly', dom, hourNum, minNum }

  return null
}

const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/** One day number (0-7, both 0 and 7 being Sunday), or null for anything else. */
function dayName(token: string): string | null {
  return /^\d+$/.test(token) ? (DAY_NAMES[Number(token)] ?? null) : null
}

/**
 * One comma-separated part of a day-of-week field, read aloud: a day
 * (`1` is Mon) or a range (`1-5` is Mon-Fri). Anything else - a step, a name,
 * an out-of-range number - is shown as written rather than guessed at.
 */
function weekdayToken(token: string): string {
  const single = dayName(token)
  if (single !== null) return single
  const range = /^(\d+)-(\d+)$/.exec(token)
  if (range) {
    const from = dayName(range[1] ?? '')
    const to = dayName(range[2] ?? '')
    if (from !== null && to !== null) return `${from}–${to}`
  }
  return token
}

export function cronToHuman(expr: string): string {
  const shape = classifyCron(expr)
  if (!shape) return ''

  switch (shape.kind) {
    case 'hourly': {
      const every = shape.intervalHours === 1 ? 'Every hour' : `Every ${shape.intervalHours} hours`
      return shape.minNum === 0 ? every : `${every} at :${shape.minNum.toString().padStart(2, '0')}`
    }
    case 'daily':
      return `Daily at ${cronTimeToDisplay(shape.hourNum, shape.minNum)}`
    case 'weekly': {
      const days = shape.dow.split(',').map(weekdayToken)
      return `${days.join(', ')} at ${cronTimeToDisplay(shape.hourNum, shape.minNum)}`
    }
    case 'monthly':
      return `Monthly on day ${shape.dom} at ${cronTimeToDisplay(shape.hourNum, shape.minNum)}`
  }
}
