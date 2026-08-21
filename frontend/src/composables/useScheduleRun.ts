// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { runSchedule, type RunScheduleRequest } from '../api/schedules'
import { extractError } from '../utils/error'
import { useToast } from './useToast'
import type { ScheduleRow, ScheduleType } from '../types/schedule'

/**
 * Names a schedule's type in the "... started." toast, or `null` for a caller
 * that does not announce starts at all.
 */
export type ScheduleTypeLabel = (type: ScheduleType) => string

export interface ScheduleRunOptions {
  /**
   * Extra fields for the POST body. The agent page scopes the run to the host
   * whose page it is, since a schedule can target several and pressing "Run
   * now" on one must not fire a backup on the others.
   */
  body?: (schedule: ScheduleRow) => RunScheduleRequest
  /**
   * Where a failure goes. Defaults to a toast; the agent page renders it
   * inline instead, above the rows it applies to. Called with `null` when a
   * run starts, so a message left over from a previous attempt clears.
   */
  onError?: (message: string | null) => void
}

export interface ScheduleRunner {
  /** The id of the schedule currently starting, so its row can disable itself. */
  runNowLoading: Ref<number | null>
  runNow: (schedule: ScheduleRow) => Promise<void>
}

/**
 * "Run now" for a schedule row: the loading id, the POST, and the reporting
 * that goes with it. Shared by the schedules list, the repository's schedules
 * tab and the agent's, which had identical copies of this function.
 *
 * The label is the caller's, not this composable's: the sites word the verify
 * type differently ("Verify" against "Verify (extract dry-run)"), and picking
 * one is a copy decision rather than a de-duplication. `null` means the caller
 * does not announce starts - the agent tab never has, and inventing a toast
 * for it would mean a fifth copy of that same switch just to word the message.
 */
export function useScheduleRun(
  labelFor: ScheduleTypeLabel | null,
  options: ScheduleRunOptions = {},
): ScheduleRunner {
  const { success: toastSuccess, error: toastError } = useToast()
  const runNowLoading = ref<number | null>(null)
  const reportError = options.onError ?? ((message) => message !== null && toastError(message))

  async function runNow(schedule: ScheduleRow): Promise<void> {
    runNowLoading.value = schedule.id
    reportError(null)
    try {
      await runSchedule(schedule.id, options.body?.(schedule) ?? {})
      if (labelFor) toastSuccess(`${labelFor(schedule.schedule_type)} started.`)
    } catch (e: unknown) {
      reportError(extractError(e))
    } finally {
      runNowLoading.value = null
    }
  }

  return { runNowLoading, runNow }
}
