// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { useToast } from './useToast'
import type { ScheduleRow, ScheduleType } from '../types/schedule'

/** Names a schedule's type in the "... started." toast. */
export type ScheduleTypeLabel = (type: ScheduleType) => string

export interface ScheduleRunner {
  /** The id of the schedule currently starting, so its row can disable itself. */
  runNowLoading: Ref<number | null>
  runNow: (schedule: ScheduleRow) => Promise<void>
}

/**
 * "Run now" for a schedule row, with the toast reporting that goes with it.
 * Shared by the schedules list and the repository's schedules tab, which had
 * identical copies of this function.
 *
 * The label is the caller's, not this composable's: the two sites word the
 * verify type differently ("Verify" against "Verify (extract dry-run)"), and
 * picking one is a copy decision rather than a de-duplication.
 */
export function useScheduleRun(labelFor: ScheduleTypeLabel): ScheduleRunner {
  const { success: toastSuccess, error: toastError } = useToast()
  const runNowLoading = ref<number | null>(null)

  async function runNow(schedule: ScheduleRow): Promise<void> {
    runNowLoading.value = schedule.id
    try {
      await apiClient.post(`/schedules/${schedule.id}/run`, {})
      toastSuccess(`${labelFor(schedule.schedule_type)} started.`)
    } catch (e: unknown) {
      toastError(extractError(e))
    } finally {
      runNowLoading.value = null
    }
  }

  return { runNowLoading, runNow }
}
