// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { byFinishedDesc, filterSettledReports, normalizeBackupStatus } from './backupStatus'
import type { ReportRow } from '../types/report'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleHealthEntry } from './scheduleHealth'

/**
 * Where one repository stands for one agent, worst first. The order of the
 * union is the order of precedence in `buildRepoOverview`: a repository that
 * is running right now says so even if its last run failed, and one that
 * failed is not softened to "overdue" just because it also missed a run.
 */
export type RepoOverviewState = 'running' | 'failed' | 'overdue' | 'warning' | 'success' | 'none'

export interface RepoOverviewRow {
  id: number
  name: string
  state: RepoOverviewState
  /** The newest settled run to this repository, of any outcome. */
  lastRun: ReportRow | null
  /** The newest successful or warned run, i.e. the last time data actually landed. */
  lastGood: ReportRow | null
  /** Soonest upcoming run of an enabled schedule writing to this repository. */
  nextRunAt: string | null
  /** This repository's settled runs, newest first, for its own outcome strip. */
  reports: ReportRow[]
}

export interface RepoOverviewInput {
  repos: readonly { id: number; name: string }[]
  reports: readonly ReportRow[]
  schedules: readonly ScheduleRow[]
  health: readonly ScheduleHealthEntry[]
  /** Repositories with a backup in flight or queued for this agent. */
  runningRepoIds: readonly number[]
}

function stateFor(
  running: boolean,
  lastRun: ReportRow | null,
  overdue: boolean,
): RepoOverviewState {
  if (running) return 'running'
  const status = lastRun ? normalizeBackupStatus(lastRun.status) : null
  if (status === 'failed') return 'failed'
  if (overdue) return 'overdue'
  if (status === 'warning') return 'warning'
  if (status === 'success') return 'success'
  return 'none'
}

/**
 * One row per repository the agent backs up to. Disabled schedules are
 * skipped for the next run: the server still reports a `next_run_at` for
 * them, but it will not fire.
 */
export function buildRepoOverview(input: RepoOverviewInput): RepoOverviewRow[] {
  const settled = filterSettledReports(input.reports).sort(byFinishedDesc)
  return input.repos.map((repo) => {
    const reports = settled.filter((r) => r.repo_id === repo.id)
    const lastRun = reports[0] ?? null
    const lastGood =
      reports.find((r) => {
        const status = normalizeBackupStatus(r.status)
        return status === 'success' || status === 'warning'
      }) ?? null
    const nextRunAt =
      input.schedules
        .filter((s) => s.enabled && s.repo_id === repo.id && s.next_run_at !== null)
        .map((s) => s.next_run_at as string)
        .sort((a, b) => new Date(a).getTime() - new Date(b).getTime())[0] ?? null
    const overdue = input.health.some((h) => h.repo_id === repo.id && h.is_overdue)
    return {
      id: repo.id,
      name: repo.name,
      state: stateFor(input.runningRepoIds.includes(repo.id), lastRun, overdue),
      lastRun,
      lastGood,
      nextRunAt,
      reports,
    }
  })
}
