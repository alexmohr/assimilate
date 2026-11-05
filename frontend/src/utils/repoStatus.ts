// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import { actionForHealth, actionLabel, quotaCeiling, quotaHealth } from './quota'
import type { RepoWithStats } from '../types/repo'

/** Why a repository row reads the way it does, most pressing first. */
export type RepoRowStatusKind =
  | 'import_failed'
  | 'importing'
  | 'disabled'
  | 'over_quota'
  | 'near_quota'
  | 'unmatched'
  | 'healthy'

/** The one status a repository row shows: a badge on a wide screen, a line on a phone. */
export interface RepoRowStatus {
  kind: RepoRowStatusKind
  label: string
  tone: BadgeTone
  /** The longer explanation behind the label, for the badge's tooltip. */
  detail: string | null
}

/** Whether the status needs looking at, so the row sorts and tints as a problem. */
export function isRepoProblem(status: RepoRowStatus): boolean {
  return status.tone === 'danger' || status.tone === 'warning'
}

function importLabel(repo: RepoWithStats): string {
  const verb = (repo.import_status_message ?? '').startsWith('Indexing') ? 'Indexing' : 'Importing'
  return repo.import_total > 0 ? `${verb} ${repo.import_progress}/${repo.import_total}` : `${verb}…`
}

/**
 * The single status a repository row shows. A repository can be several
 * things at once (disabled and over its quota); the row names the one that
 * most needs attention, in the order of the cases below.
 */
export function repoRowStatus(repo: RepoWithStats): RepoRowStatus {
  if (repo.import_error) {
    return {
      kind: 'import_failed',
      label: 'Import failed',
      tone: 'danger',
      detail: repo.import_error,
    }
  }
  if (repo.importing) {
    return {
      kind: 'importing',
      label: importLabel(repo),
      tone: 'warning',
      detail: repo.import_status_message,
    }
  }
  if (!repo.enabled) {
    return { kind: 'disabled', label: 'Disabled', tone: 'neutral', detail: null }
  }
  const health = quotaHealth(repo.quota, repo.total_deduplicated_size)
  if (repo.quota && (health === 'critical' || health === 'warning')) {
    const action = actionForHealth(health, repo.quota.warn_action, repo.quota.critical_action)
    const detail = action ? actionLabel(action) : null
    return health === 'critical'
      ? { kind: 'over_quota', label: 'Over quota', tone: 'danger', detail }
      : { kind: 'near_quota', label: 'Near quota', tone: 'warning', detail }
  }
  if (repo.unmatched_count > 0) {
    return {
      kind: 'unmatched',
      label: `${repo.unmatched_count} unmatched`,
      tone: 'warning',
      detail: 'Imported archives no schedule claims yet',
    }
  }
  return { kind: 'healthy', label: 'Healthy', tone: 'success', detail: null }
}

/** A repository's usage against its own quota, or null when it has none to measure against. */
export interface RepoQuotaUse {
  ceilingBytes: number
  /** Usage as a share of the ceiling, clipped to 0-100 for drawing. */
  fillPercent: number
}

export function repoQuotaUse(repo: RepoWithStats): RepoQuotaUse | null {
  if (!repo.quota?.enabled) return null
  const ceiling = quotaCeiling(repo.quota)
  if (ceiling === null || ceiling <= 0) return null
  return {
    ceilingBytes: ceiling,
    fillPercent: Math.min(100, (repo.total_deduplicated_size / ceiling) * 100),
  }
}
