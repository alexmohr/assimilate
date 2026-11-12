// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { RouteLocationRaw } from 'vue-router'

/** What a link into the Activity log narrows it to; every field is optional. */
export interface RunLogTarget {
  hostname?: string | null
  scheduleId?: number | null
  /** The repository's name, as the log's target filter matches it. */
  target?: string | null
  status?: 'failed' | 'warning' | null
  /** One run's report, which the log opens on arrival so its output shows at once. */
  reportId?: number | null
}

/**
 * Where a failed or warned run is read: the Activity log, filtered to it and,
 * given its report, with that run already open. Every "failed" link points
 * here rather than at the repository or schedule, which only say that it
 * failed, not why.
 */
export function runLogLink(run: RunLogTarget): RouteLocationRaw {
  const query: Record<string, string> = { category: 'backup' }
  if (run.hostname) query.hostname = run.hostname
  if (run.scheduleId) query.schedule_id = String(run.scheduleId)
  if (run.target) query.target = run.target
  if (run.status) query.status = run.status
  if (run.reportId) query.report = String(run.reportId)
  return { path: '/activity', query }
}
