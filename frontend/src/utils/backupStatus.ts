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
