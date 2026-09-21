// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ReportRow } from '../types/report'
import { filterSettledReports, normalizeBackupStatus } from './backupStatus'
import type { NormalizedBackupStatus } from './backupStatus'
import type { ScheduleRepoOption } from '../types/schedule'

/**
 * A schedule's runs, split by the repository they wrote into.
 *
 * A schedule copies its sources into an ordered list of repositories and each
 * target is its own borg run, so one occurrence leaves one report per
 * repository behind. Merged into a single list - which is how the schedule
 * screen read them - a run of two copies where only the offsite one failed is
 * indistinguishable from one where both did, and from one host failing
 * outright. Splitting them is what lets every screen say which copy is which.
 */
export interface ScheduleRepoRuns {
  repo: ScheduleRepoOption
  /** Settled runs against this repository, newest first. */
  reports: ReportRow[]
  /** The newest settled run, or null when this repository has never finished one. */
  last: ReportRow | null
  /** The newest settled run's outcome, or null when there is none. */
  status: NormalizedBackupStatus | null
}

/**
 * Newest first. The reports endpoint orders its rows already, but this is
 * also fed the schedule detail view's accumulated pages, so it sorts rather
 * than trusting the order it is handed.
 */
function byFinishedDesc(a: ReportRow, b: ReportRow): number {
  return new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime()
}

/**
 * Groups settled reports by repository, one entry per *target* - in write
 * order, and including a target that has never run, which is itself worth
 * seeing on a status screen.
 *
 * Reports whose `repo_id` is not a current target are dropped: a repository
 * removed from the schedule since leaves its old runs behind, and listing
 * them under a heading the schedule no longer writes to would report a
 * failure nobody can act on.
 *
 * Deliberately per repository and not per host: a schedule with several agent
 * targets writes every one of them into the same repositories, and this answers
 * "is this copy current", which is a question about the repository. The
 * consequence is that on such a schedule the newest run wins regardless of which
 * host produced it, so one host's failure can sit behind another's later
 * success here. That is why the per-host reading is a separate function -
 * `failingRepoCount` below, which the Targets rows use - rather than something
 * callers are expected to recover from these entries.
 */
export function scheduleRepoRuns(
  repos: readonly ScheduleRepoOption[],
  reports: readonly ReportRow[],
): ScheduleRepoRuns[] {
  const settled = filterSettledReports(reports)
  return repos.map((repo) => {
    const own = settled.filter((r) => r.repo_id === repo.id).sort(byFinishedDesc)
    const last = own[0] ?? null
    return {
      repo,
      reports: own,
      last,
      status: last ? normalizeBackupStatus(last.status) : null,
    }
  })
}

/**
 * How many of a schedule's repositories ended their most recent run *for this
 * host* in failure.
 *
 * Per host rather than per schedule: a target row speaks for one machine, and
 * another host's failed copy is not that row's problem. A repository this
 * host has never run against counts as neither failing nor healthy.
 */
export function failingRepoCount(runs: readonly ScheduleRepoRuns[], agentId: number): number {
  return runs.filter((entry) => {
    const latest = entry.reports.find((r) => r.agent_id === agentId)
    return latest != null && normalizeBackupStatus(latest.status) === 'failed'
  }).length
}
