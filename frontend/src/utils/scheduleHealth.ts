// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Router } from 'vue-router'
import { normalizeBackupStatus, type NormalizedBackupStatus } from './backupStatus'
import type { EntityIssue } from '../components/EntityStatusBadges.vue'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'

// The generated wire type (`shared::responses::HealthResponse`) is the
// single source of truth - shared by every view that reads `/stats/health`
// for schedule-level health (SchedulesView, AgentDetailView).
export type ScheduleHealthEntry = HealthSummaryResponse

export function scheduleRunStatus(
  entry: ScheduleHealthEntry | null | undefined,
): NormalizedBackupStatus | null {
  return entry?.last_status != null ? normalizeBackupStatus(entry.last_status) : null
}

export function navigateToScheduleIssue(
  router: Router,
  scheduleId: number,
  kind: 'failed' | 'warning' | 'overdue',
): void {
  if (kind === 'overdue') {
    router.push(`/schedules/${scheduleId}`)
    return
  }
  router.push(`/activity?category=backup&schedule_id=${scheduleId}&status=${kind}`)
}

/**
 * Chips for a schedule card's badge row, derived from every health entry
 * belonging to that schedule (a schedule can have more than one target/host,
 * each independently overdue/failed/warning).
 */
export function scheduleIssuesFromEntries(
  entries: ScheduleHealthEntry[],
  scheduleId: number,
  router: Router,
): EntityIssue[] {
  const issues: EntityIssue[] = []
  if (entries.some((h) => h.is_overdue)) {
    issues.push({
      key: 'overdue',
      label: 'Overdue',
      severity: 'warning',
      onClick: () => navigateToScheduleIssue(router, scheduleId, 'overdue'),
    })
  }
  if (entries.some((h) => scheduleRunStatus(h) === 'failed')) {
    issues.push({
      key: 'failed',
      label: 'Failed',
      severity: 'danger',
      onClick: () => navigateToScheduleIssue(router, scheduleId, 'failed'),
    })
  } else if (entries.some((h) => scheduleRunStatus(h) === 'warning')) {
    issues.push({
      key: 'warning',
      label: 'Warning',
      severity: 'warning',
      onClick: () => navigateToScheduleIssue(router, scheduleId, 'warning'),
    })
  }
  const missedIssue = missedBackupsIssue(entries[0], scheduleId, router)
  if (missedIssue) {
    issues.push(missedIssue)
  }
  return issues
}

/**
 * A schedule that keeps missing its scheduled runs (agent or target
 * unreachable at trigger time) shows a warning as soon as it misses one, below
 * the schedule's own `missed_backup_threshold` - once it reaches that count the
 * scheduler auto-disables the schedule, which already surfaces via
 * `scheduleDisabledLabel()`'s "Auto-disabled" pill, so this chip only covers
 * the "still retrying" range.
 */
function missedBackupsIssue(
  entry: ScheduleHealthEntry | undefined,
  scheduleId: number,
  router: Router,
): EntityIssue | null {
  if (entry == null) return null
  const { consecutive_missed_backups: missed, missed_backup_threshold: threshold } = entry
  if (missed <= 0 || missed >= threshold) return null
  return {
    key: 'missed-backups',
    label: `${missed}/${threshold} missed`,
    severity: 'warning',
    title: `${missed} consecutive missed backup${missed === 1 ? '' : 's'} (agent or target unreachable) - will be marked failed at ${threshold}`,
    onClick: () => router.push(`/schedules/${scheduleId}`),
  }
}

/**
 * Attaches a hover-detail `title` to each non-overdue issue, taken from the
 * matching health entry's `last_error_message`. Shared by every view that
 * renders `scheduleIssuesFromEntries()`'s output; overdue issues are left
 * untouched since each caller has its own (or no) overdue-title logic.
 */
export function withErrorTitles(
  issues: EntityIssue[],
  entries: ScheduleHealthEntry[],
): EntityIssue[] {
  return issues.map((issue) => {
    if (issue.key === 'overdue') return issue
    const entry = entries.find((h) => scheduleRunStatus(h) === issue.key)
    return entry?.last_error_message ? { ...issue, title: entry.last_error_message } : issue
  })
}
