// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { normalizeBackupStatus } from './backupStatus'
import type { ReportRow } from '../types/report'

/**
 * The reports in a run log that can be retried: a failed run of a schedule
 * that is still the newest run of its (schedule, host, repository) copy.
 *
 * Only the newest, because a retry is for "the last attempt broke" - offering
 * it on a failure from last month that has since succeeded would re-run a
 * backup nobody needs, and once a retry is queued its pending row is newer,
 * so the button goes away by itself instead of inviting a second press. A
 * run with no schedule (an import, a one-off) has nothing to re-run.
 *
 * "Newest" is the largest id among the rows loaded: ids are handed out in
 * insertion order and a pending run has no meaningful `finished_at` yet.
 */
export function retryableReportIds(reports: readonly ReportRow[]): Set<number> {
  const newest = new Map<string, ReportRow>()
  reports.forEach((report) => {
    if (report.schedule_id === null) return
    const key = `${report.schedule_id}:${report.agent_id}:${report.repo_id}`
    const current = newest.get(key)
    if (!current || report.id > current.id) newest.set(key, report)
  })
  return new Set(
    [...newest.values()]
      .filter((report) => normalizeBackupStatus(report.status) === 'failed')
      .map((report) => report.id),
  )
}
