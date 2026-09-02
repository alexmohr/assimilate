// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { extractError } from '../utils/error'
import type { ReportRow } from '../types/report'

/** Matches the server default so a first page here looks the same as a bare fetch elsewhere. */
export const REPORTS_PAGE_SIZE = 50

export interface ReportsPage {
  reports: ReportRow[]
  total: number
}

export interface ReportsPager {
  reports: Ref<ReportRow[]>
  total: Ref<number>
  loading: Ref<boolean>
  loadingMore: Ref<boolean>
  error: Ref<string | null>
  /** Replaces the list with a fresh first page. */
  load: () => Promise<void>
  /** Appends the next page after what is already loaded. */
  loadMore: () => Promise<void>
}

/**
 * Pages a backup-report list backed by a `{ reports, total }` endpoint (an
 * agent's or a schedule's), used by every tab that renders `RunLogTab`.
 *
 * The server has always capped a bare `/reports` fetch at 50 rows; before
 * this there was no `total` in the response, so a tab showing "50" had no
 * way to say whether that was everything or just where the cap happened to
 * land. `load` replaces the list (used on mount and on any change that could
 * invalidate it, e.g. a WebSocket `DataChanged`); `loadMore` appends the next
 * page, keyed off how many rows are already loaded rather than a page index,
 * so a concurrent insert can't leave a gap or a duplicate at the boundary.
 */
export function useReportsPager(
  fetchPage: (limit: number, offset: number) => Promise<ReportsPage>,
): ReportsPager {
  const reports = ref<ReportRow[]>([])
  const total = ref(0)
  const loading = ref(false)
  const loadingMore = ref(false)
  const error = ref<string | null>(null)

  async function load(): Promise<void> {
    loading.value = true
    error.value = null
    try {
      const page = await fetchPage(REPORTS_PAGE_SIZE, 0)
      reports.value = page.reports
      total.value = page.total
    } catch (e: unknown) {
      error.value = extractError(e)
    } finally {
      loading.value = false
    }
  }

  async function loadMore(): Promise<void> {
    loadingMore.value = true
    error.value = null
    try {
      const page = await fetchPage(REPORTS_PAGE_SIZE, reports.value.length)
      reports.value = [...reports.value, ...page.reports]
      total.value = page.total
    } catch (e: unknown) {
      error.value = extractError(e)
    } finally {
      loadingMore.value = false
    }
  }

  return { reports, total, loading, loadingMore, error, load, loadMore }
}
