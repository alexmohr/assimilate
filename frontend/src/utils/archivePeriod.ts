// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { getConfiguredTimezone } from '../composables/useTimezone'

/**
 * The stretch of time an archive's list groups it under when grouped by day:
 * today, yesterday, the rest of the last week, then one group per month.
 *
 * Coarser than one group per day on purpose: a daily schedule's fourteen
 * archives would otherwise be fourteen headings of one row each.
 */
export interface ArchivePeriod {
  /** Stable across renders, for keys and collapse state. */
  key: string
  label: string
}

const DAY_MS = 86_400_000

/** The calendar day `date` falls on in the configured timezone, as a UTC midnight. */
function calendarDay(date: Date, timeZone: string | undefined): number {
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(date)
  const part = (type: Intl.DateTimeFormatPartTypes): number =>
    Number(parts.find((p) => p.type === type)?.value ?? 0)
  return Date.UTC(part('year'), part('month') - 1, part('day'))
}

export function archivePeriod(start: string, now: Date = new Date()): ArchivePeriod {
  const timeZone = getConfiguredTimezone()
  const date = new Date(start)
  const daysAgo = Math.round((calendarDay(now, timeZone) - calendarDay(date, timeZone)) / DAY_MS)
  if (daysAgo <= 0) return { key: 'today', label: 'Today' }
  if (daysAgo === 1) return { key: 'yesterday', label: 'Yesterday' }
  if (daysAgo < 7) return { key: 'week', label: 'This week' }
  const label = date.toLocaleDateString(undefined, { timeZone, month: 'long', year: 'numeric' })
  const month = new Intl.DateTimeFormat('en-CA', { timeZone, year: 'numeric', month: '2-digit' })
    .format(date)
    .slice(0, 7)
  return { key: `month-${month}`, label }
}
