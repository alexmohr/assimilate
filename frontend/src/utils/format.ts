// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { getConfiguredTimezone } from '../composables/useTimezone'

export function formatBytes(bytes: number | null | undefined): string {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`
}

export function formatDate(iso: string | null, fallback: string = '\u2014'): string {
  if (!iso) return fallback
  return new Date(iso).toLocaleString(undefined, {
    timeZone: getConfiguredTimezone(),
  })
}

export function formatDateShort(iso: string | null, fallback: string = '\u2014'): string {
  if (!iso) return fallback
  return new Date(iso).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZone: getConfiguredTimezone(),
  })
}

export function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`
  const m = Math.floor(secs / 60)
  const s = secs % 60
  if (m < 60) return `${m}m ${s}s`
  const h = Math.floor(m / 60)
  const rm = m % 60
  return `${h}h ${rm}m`
}

export function relativeTime(iso: string): string {
  const ts = new Date(iso).getTime()
  if (isNaN(ts) || ts === 0) return 'Never'
  const diff = Date.now() - ts
  if (diff >= 0) {
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return 'just now'
    if (mins < 60) return `${mins}m ago`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}h ago`
    return `${Math.floor(hrs / 24)}d ago`
  }
  const futureMins = Math.floor(-diff / 60000)
  if (futureMins < 1) return 'in < 1m'
  if (futureMins < 60) return `in ${futureMins}m`
  const futureHrs = Math.floor(futureMins / 60)
  if (futureHrs < 24) return `in ${futureHrs}h`
  return `in ${Math.floor(futureHrs / 24)}d`
}
