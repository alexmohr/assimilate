// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { classifyCron } from './cron'

const SECONDS_PER_HOUR = 3600
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR
const SECONDS_PER_WEEK = 7 * SECONDS_PER_DAY
const SECONDS_PER_MONTH_APPROX = 30 * SECONDS_PER_DAY

/**
 * Approximate seconds between runs for the cron shapes `cronToHuman`
 * understands (hourly `* / N`, daily, weekly on one or more weekdays,
 * monthly on a day-of-month). Returns null for anything else, since there is
 * no single fixed interval to report - callers treat that as "cadence
 * unknown" rather than guessing.
 */
export function cronIntervalSecs(expr: string): number | null {
  const shape = classifyCron(expr)
  if (!shape) return null

  switch (shape.kind) {
    case 'hourly':
      return shape.intervalHours * SECONDS_PER_HOUR
    case 'daily':
      return SECONDS_PER_DAY
    case 'weekly':
      return Math.round(SECONDS_PER_WEEK / shape.dow.split(',').length)
    case 'monthly':
      return SECONDS_PER_MONTH_APPROX
  }
}
