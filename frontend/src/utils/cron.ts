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
 * different projections of the same expression - a human-readable string
 * here, an approximate interval in `cadence.ts` - don't each re-implement
 * field parsing and shape detection and risk drifting out of sync.
 */
export type CronShape =
  | { kind: 'hourly'; intervalHours: number }
  | { kind: 'daily'; hourNum: number; minNum: number }
  | { kind: 'weekly'; dow: string; hourNum: number; minNum: number }
  | { kind: 'monthly'; dom: string; hourNum: number; minNum: number }

export function classifyCron(expr: string): CronShape | null {
  const parts = expr.trim().split(/\s+/)
  if (parts.length !== 5) return null

  const [min, hour, dom, , dow] = parts

  const hourlyMatch = hour.match(/^\*\/(\d+)$/)
  if (hourlyMatch && min === CRON_TOP_OF_HOUR && dom === CRON_ANY && dow === CRON_ANY) {
    return { kind: 'hourly', intervalHours: parseInt(hourlyMatch[1], 10) }
  }

  const minNum = parseInt(min, 10)
  const hourNum = parseInt(hour, 10)
  if (isNaN(minNum) || isNaN(hourNum)) return null

  if (dom === CRON_ANY && dow === CRON_ANY) return { kind: 'daily', hourNum, minNum }
  if (dom === CRON_ANY && dow !== CRON_ANY) return { kind: 'weekly', dow, hourNum, minNum }
  if (dow === CRON_ANY && dom !== CRON_ANY) return { kind: 'monthly', dom, hourNum, minNum }

  return null
}

export function cronToHuman(expr: string): string {
  const shape = classifyCron(expr)
  if (!shape) return ''

  switch (shape.kind) {
    case 'hourly':
      return shape.intervalHours === 1 ? 'Every hour' : `Every ${shape.intervalHours} hours`
    case 'daily':
      return `Daily at ${cronTimeToDisplay(shape.hourNum, shape.minNum)}`
    case 'weekly': {
      const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
      const days = shape.dow.split(',').map((d) => {
        const n = parseInt(d, 10)
        return dayNames[n] ?? d
      })
      return `${days.join(', ')} at ${cronTimeToDisplay(shape.hourNum, shape.minNum)}`
    }
    case 'monthly':
      return `Monthly on day ${shape.dom} at ${cronTimeToDisplay(shape.hourNum, shape.minNum)}`
  }
}
