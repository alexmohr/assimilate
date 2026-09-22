// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * Normalized backup outcome, shared across views that render report/activity
 * status. The wire type (`ReportResponse.status`, `ActivityRow.status`, ...)
 * is a plain `string` since the backend serializes `BackupStatus` via
 * `Display`, so every reader has to parse it into this union at the boundary
 * rather than repeating raw string comparisons.
 */
export type NormalizedBackupStatus =
  | 'success'
  | 'warning'
  | 'failed'
  | 'started'
  | 'pending'
  | 'cancelled'

export function normalizeBackupStatus(rawStatus: string): NormalizedBackupStatus {
  const s = rawStatus.toLowerCase()
  if (s === 'success') return 'success'
  if (s === 'warning') return 'warning'
  if (s === 'started') return 'started'
  if (s === 'pending') return 'pending'
  if (s === 'cancelled') return 'cancelled'
  return 'failed'
}

/**
 * Reports with an outcome. A pending or started run hasn't finished, so it
 * has nothing to show in a "recent backups" list yet - the agent and
 * schedule Overview tabs both filter to this before slicing a preview.
 */
export function filterSettledReports<T extends { status: string }>(reports: readonly T[]): T[] {
  return reports.filter((r) => {
    const status = normalizeBackupStatus(r.status)
    return status !== 'pending' && status !== 'started'
  })
}

/**
 * Newest finished run first, with the report id breaking ties.
 *
 * Every screen that shows "the last run" or a recent-runs list orders by this,
 * so it lives here rather than being written out at each of them - a
 * comparator copied per screen is one that can drift per screen, and two
 * screens disagreeing about which of two runs is newer is a bug nobody would
 * think to look for.
 *
 * The tie-break is what makes "newest" an answer rather than a coincidence:
 * `finished_at` has one-second resolution, so a retry that lands in the same
 * second as the run it replaces compares equal, and `Array.sort` is stable -
 * leaving the order the caller happened to receive to decide. Ids are handed
 * out in insertion order, so the larger one is the later run.
 */
export function byFinishedDesc<T extends { finished_at: string; id: number }>(a: T, b: T): number {
  const diff = new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime()
  return diff !== 0 ? diff : b.id - a.id
}

/**
 * What a run has to *say*: its warnings, or the error that ended it. The
 * label doubles as the predicate - null means there is nothing to read, so
 * a preview row offers no jump to it.
 *
 * Warnings win over the error message because a warned run carries both: the
 * agent fills `error_message` for a warning-only run too (the notification
 * path reads it), and the detail block renders the warnings rather than
 * repeating them as an error.
 *
 * Shared rather than repeated: this decides which button a viewer sees, on
 * the host Overview (AgentBackupRow) and the schedule Overview alike, and
 * the two drifting apart would show different rows for the same run.
 */
export type ReportMessageLabel = 'View warnings' | 'View error'

export function reportMessageLabel(report: {
  status: string
  error_message: string | null
  /** The wire type says `Array<string>`, but a report can reach the UI without it. */
  warnings?: readonly string[] | null
}): ReportMessageLabel | null {
  if (report.warnings && report.warnings.length > 0) return 'View warnings'
  if (report.error_message && normalizeBackupStatus(report.status) !== 'success') {
    return 'View error'
  }
  return null
}
