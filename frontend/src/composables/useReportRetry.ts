// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { runSchedule } from '../api/schedules'
import { extractError } from '../utils/error'
import { useToast } from './useToast'
import type { ReportRow } from '../types/report'

export interface ReportRetry {
  /** The report whose retry is being sent, so its row can disable its button. */
  retryingReportId: Ref<number | null>
  retry: (report: ReportRow) => Promise<void>
}

/**
 * "Retry" on a failed row of a run log: runs the report's schedule again for
 * that one host and that one repository, so the copies that succeeded are not
 * written twice. The server checks permission on the repository and queues
 * the run even when the host is offline, hence "queued" rather than "started".
 *
 * `onQueued` reloads the log: the pending row the server inserted before it
 * answered is what replaces the button, since a failed row stops being
 * retryable as soon as a newer run of its copy exists.
 */
export function useReportRetry(onQueued: () => Promise<void> | void): ReportRetry {
  const { success: toastSuccess, error: toastError } = useToast()
  const retryingReportId = ref<number | null>(null)

  async function retry(report: ReportRow): Promise<void> {
    if (report.schedule_id === null || retryingReportId.value !== null) return
    retryingReportId.value = report.id
    try {
      await runSchedule(report.schedule_id, {
        agent_ids: [report.agent_id],
        repo_ids: [report.repo_id],
      })
      const repo = report.repo_name ?? 'the repository'
      const host = report.hostname ?? 'the host'
      toastSuccess(`Retry queued for ${repo} on ${host}.`)
      await onQueued()
    } catch (e: unknown) {
      toastError(extractError(e))
    } finally {
      retryingReportId.value = null
    }
  }

  return { retryingReportId, retry }
}
