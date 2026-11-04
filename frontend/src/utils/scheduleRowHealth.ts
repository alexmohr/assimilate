// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import { missedBackupStreak, scheduleRunStatus, type ScheduleHealthEntry } from './scheduleHealth'
import { catchUpPendingTitle } from './scheduleStatus'

/** Everything the Schedules list knows about one schedule's state. */
export interface ScheduleRowHealthInput {
  enabled: boolean
  /** `scheduleDisabledLabel()`: "Disabled", or why it was disabled automatically. */
  disabledLabel: string
  /** Disabled by the scheduler after repeated misses or errors, not by a person. */
  autoDisabled: boolean
  running: boolean
  /** Every `/stats/health` entry of the schedule, one per target. */
  entries: readonly ScheduleHealthEntry[]
  /** Which targets are overdue and when each last backed up, one per line. */
  overdueTitle: string
  /** Target agents that are currently disconnected, with when they were last seen. */
  offlineAgents: readonly string[]
  catchUpPendingCount: number
  /** Agents whose runs write to the same repository at the same time as this one's. */
  overlapAgents: readonly string[]
}

/** The one chip a schedule row shows, and the status dot beside its name. */
export interface ScheduleRowHealth {
  label: string
  tone: BadgeTone
  /** Hover detail; empty when the label says it all. */
  title: string
  /** Needs attention: sorts the row to the top of its group. */
  problem: boolean
}

/**
 * Folds a schedule's state into a single chip. Problems that can coexist -
 * overdue, a failed or warned run, missed runs, an overlap with another agent
 * - are joined into one label ("Overdue · 1/3 missed") so a row never carries
 * a row of chips; otherwise the most telling state wins: running, disabled,
 * an offline agent, a pending catch-up, and finally Healthy.
 */
export function scheduleRowHealth(input: ScheduleRowHealthInput): ScheduleRowHealth {
  if (input.running) {
    return { label: 'Running', tone: 'accent', title: '', problem: false }
  }
  if (!input.enabled) {
    return {
      label: input.disabledLabel,
      tone: input.autoDisabled ? 'danger' : 'neutral',
      title: '',
      problem: input.autoDisabled,
    }
  }

  const parts: string[] = []
  const titles: string[] = []
  let tone: BadgeTone = 'warning'

  if (input.entries.some((h) => h.is_overdue)) {
    parts.push('Overdue')
    titles.push(input.overdueTitle)
  }
  const failed = input.entries.find((h) => scheduleRunStatus(h) === 'failed')
  const warned = input.entries.find((h) => scheduleRunStatus(h) === 'warning')
  if (failed) {
    parts.push('Failed')
    tone = 'danger'
    if (failed.last_error_message) titles.push(failed.last_error_message)
  } else if (warned) {
    parts.push('Warning')
    if (warned.last_error_message) titles.push(warned.last_error_message)
  }
  const missed = missedBackupStreak(input.entries[0])
  if (missed) {
    parts.push(missed.label)
    titles.push(missed.title)
  }
  for (const agent of input.overlapAgents) {
    parts.push(`Overlaps ${agent}`)
    titles.push(`Writes to the same repository as ${agent} at the same time`)
  }
  if (parts.length > 0) {
    return { label: parts.join(' · '), tone, title: titles.join('\n'), problem: true }
  }

  if (input.offlineAgents.length > 0) {
    return {
      label: 'Agent offline',
      tone: 'warning',
      title: input.offlineAgents.join('\n'),
      problem: true,
    }
  }
  if (input.catchUpPendingCount > 0) {
    return {
      label: 'Catch-up pending',
      tone: 'info',
      title: catchUpPendingTitle(input.catchUpPendingCount),
      problem: false,
    }
  }
  if (input.entries.every((h) => scheduleRunStatus(h) === null)) {
    return { label: 'Not run yet', tone: 'neutral', title: '', problem: false }
  }
  return { label: 'Healthy', tone: 'success', title: '', problem: false }
}
