// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { getConfiguredTimezone } from '../composables/useTimezone'

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

export function cronToHuman(expr: string): string {
  const parts = expr.trim().split(/\s+/)
  if (parts.length !== 5) return ''

  const [min, hour, dom, , dow] = parts

  const hourlyMatch = hour.match(/^\*\/(\d+)$/)
  if (hourlyMatch && min === '0' && dom === '*' && dow === '*') {
    const interval = parseInt(hourlyMatch[1], 10)
    return interval === 1 ? 'Every hour' : `Every ${interval} hours`
  }

  const minNum = parseInt(min, 10)
  const hourNum = parseInt(hour, 10)
  if (isNaN(minNum) || isNaN(hourNum)) return ''

  const time = cronTimeToDisplay(hourNum, minNum)

  if (dom === '*' && dow === '*') {
    return `Daily at ${time}`
  }

  if (dom === '*' && dow !== '*') {
    const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
    const days = dow.split(',').map((d) => {
      const n = parseInt(d, 10)
      return dayNames[n] ?? d
    })
    return `${days.join(', ')} at ${time}`
  }

  if (dow === '*' && dom !== '*') {
    return `Monthly on day ${dom} at ${time}`
  }

  return ''
}
