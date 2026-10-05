// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ReportRow } from '../types/report'
import { byFinishedDesc, filterSettledReports, normalizeBackupStatus } from './backupStatus'

/** A settled run's outcome: what is left of the backup status once it has finished. */
export type RunOutcome = 'success' | 'warning' | 'skipped' | 'cancelled' | 'failed'

/** Worst first, so a run reads as its most serious report. */
export const RUN_OUTCOMES: readonly RunOutcome[] = [
  'failed',
  'warning',
  'skipped',
  'cancelled',
  'success',
]

/** How a run's outcome is named on its badge and its pill. */
export const RUN_OUTCOME_LABELS: Record<RunOutcome, string> = {
  success: 'Succeeded',
  warning: 'Warning',
  skipped: 'Skipped',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

/** How many runs the strip draws, matching the agent Overview's run strip. */
export const SCHEDULE_RUN_WINDOW = 20

/**
 * One occurrence of a schedule: every report it left behind, one per host and
 * repository it wrote into.
 */
export interface ScheduleRun {
  /** The run id, or the report's own id for a report that carries none. */
  key: string
  /** Its reports, in the order they finished. */
  reports: ReportRow[]
  /** The worst outcome among them. */
  status: RunOutcome
  /** When its last report finished. */
  finishedAt: string
  /** How long it took, summed over its reports, which run one after another. */
  durationSecs: number
}

export function runOutcome(rawStatus: string): RunOutcome {
  const status = normalizeBackupStatus(rawStatus)
  // Only settled reports reach a run, so an in-flight status cannot occur;
  // reading it as a failure keeps the type total without hiding anything.
  return status === 'pending' || status === 'started' ? 'failed' : status
}

/** The most serious outcome among a run's reports. */
export function worstOutcome(reports: readonly ReportRow[]): RunOutcome {
  const outcomes = new Set(reports.map((r) => runOutcome(r.status)))
  return RUN_OUTCOMES.find((o) => outcomes.has(o)) ?? 'success'
}

/**
 * The run a report belongs to. A report from before runs were recorded, or one
 * the repository sync imported, has no run id and stands on its own.
 */
export function runKey(report: ReportRow): string {
  return report.run_id ?? `report-${report.id}`
}

/**
 * Groups a schedule's settled reports into runs, newest first, keeping the
 * newest `limit`. A pending or started report has no outcome yet and is left
 * out, so the newest pill does not flicker while a backup is in flight.
 */
export function groupScheduleRuns(
  reports: readonly ReportRow[],
  limit: number = SCHEDULE_RUN_WINDOW,
): ScheduleRun[] {
  const byKey = new Map<string, ReportRow[]>()
  for (const report of [...filterSettledReports(reports)].sort(byFinishedDesc)) {
    const key = runKey(report)
    const group = byKey.get(key)
    if (group) group.push(report)
    else byKey.set(key, [report])
  }
  // A Map keeps insertion order, and each run was inserted at its newest
  // report - so the runs are already newest first.
  return [...byKey.entries()].slice(0, limit).map(([key, newestFirst]) => {
    const reportsInOrder = [...newestFirst].reverse()
    return {
      key,
      reports: reportsInOrder,
      status: worstOutcome(reportsInOrder),
      finishedAt: newestFirst[0]?.finished_at ?? '',
      durationSecs: reportsInOrder.reduce((sum, r) => sum + r.duration_secs, 0),
    }
  })
}

/** Runs per outcome, for the legend. */
export function countOutcomes(runs: readonly ScheduleRun[]): Record<RunOutcome, number> {
  const counts: Record<RunOutcome, number> = {
    success: 0,
    warning: 0,
    skipped: 0,
    cancelled: 0,
    failed: 0,
  }
  for (const run of runs) counts[run.status]++
  return counts
}

/** A run that wrote its archives, warnings or not. */
export function isCompleted(run: ScheduleRun): boolean {
  return run.status === 'success' || run.status === 'warning'
}

/**
 * The report a run's "Open in Logs" lands on: the first one that did not
 * succeed, since that is what someone opening a broken run wants to read.
 */
export function reportToOpen(run: ScheduleRun): ReportRow | null {
  return run.reports.find((r) => runOutcome(r.status) !== 'success') ?? run.reports[0] ?? null
}
