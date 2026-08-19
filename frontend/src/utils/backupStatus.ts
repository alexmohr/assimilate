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
