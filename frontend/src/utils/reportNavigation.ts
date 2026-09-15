// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Router } from 'vue-router'
import type { ReportRow } from '../types/report'

/**
 * Navigates to a successful report's archive: the repository's own Backups
 * tab (the archive browser), pre-selecting the archive the report wrote if
 * one was recorded. Shared by the agent and schedule detail pages, whose
 * preview rows both link a report to the same place.
 */
export function openReportArchive(router: Router, r: ReportRow): void {
  const query: Record<string, string> = { tab: 'archives' }
  if (r.archive_name) {
    query.archive = r.archive_name
  }
  router.push({ path: `/repos/${r.repo_id}`, query })
}
