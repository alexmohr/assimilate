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
 * invalidate it, e.g. a WebSocket `DataChanged`) - refetching at least as
 * many rows as were already showing, not a flat first page, so a live
 * refresh triggered by an unrelated event elsewhere doesn't undo a user's
 * "Load more" progress. `loadMore` appends the next page, keyed off how many
 * rows are already loaded rather than a page index - this only protects
 * against a same-`started_at` tie at the boundary (paired with the `id DESC`
 * secondary sort server-side); a genuinely new row inserted with a newer
 * `started_at` between two `loadMore()` calls still shifts every
 * already-loaded row's rank by one, so the next page can re-return the row
 * already shown last. Low-impact (a duplicate list entry, self-heals on the
 * next `load()`) and not fixed here - true gap/duplicate-free paging would
 * need a cursor keyed off the last loaded row's `(started_at, id)` rather
 * than a raw row count.
 */
export function useReportsPager(
  fetchPage: (limit: number, offset: number) => Promise<ReportsPage>,
): ReportsPager {
  const reports = ref<ReportRow[]>([])
  const total = ref(0)
  const loading = ref(false)
  const loadingMore = ref(false)
  const error = ref<string | null>(null)

  // A caller (e.g. the initial page-load fetch racing a just-clicked Run
  // now's own refresh) can have two `load()` calls in flight together; with
  // nothing to order them, whichever response happens to land second wins,
  // even if it started first and is now stale. Each call stamps the request
  // it started with a token and only applies the response if it is still the
  // most recent call made - a later call's own apply always wins instead.
  let loadToken = 0

  async function load(): Promise<void> {
    const token = ++loadToken
    loading.value = true
    // A fresh load() replaces the list wholesale, so any loadMore() still in
    // flight is now moot: its own token check below will discard its
    // response, but that leaves its `finally` block unable to match tokens
    // and clear this flag - without resetting it here, "Load more" would
    // stay disabled until the component remounts.
    loadingMore.value = false
    error.value = null
    // Re-fetches at least as many rows as are already showing, not a flat
    // first page. load() isn't only the initial fetch - it's also what a
    // live WebSocket DataChanged refresh runs on every backup event
    // fleet-wide (see AgentDetailView's fetchAgent/loadTabData and
    // ScheduleDetailView's useArchiveDeletionEvents), and a flat
    // REPORTS_PAGE_SIZE there would silently collapse a user's "Load more"
    // progress back to page 1 on the next unrelated event, with nothing on
    // screen to explain why.
    const limit = Math.max(reports.value.length, REPORTS_PAGE_SIZE)
    try {
      const page = await fetchPage(limit, 0)
      if (token !== loadToken) return
      reports.value = page.reports
      total.value = page.total
    } catch (e: unknown) {
      if (token !== loadToken) return
      error.value = extractError(e)
    } finally {
      if (token === loadToken) loading.value = false
    }
  }

  async function loadMore(): Promise<void> {
    const token = ++loadToken
    loadingMore.value = true
    error.value = null
    try {
      const page = await fetchPage(REPORTS_PAGE_SIZE, reports.value.length)
      if (token !== loadToken) return
      reports.value = [...reports.value, ...page.reports]
      total.value = page.total
    } catch (e: unknown) {
      if (token !== loadToken) return
      error.value = extractError(e)
    } finally {
      if (token === loadToken) loadingMore.value = false
    }
  }

  return { reports, total, loading, loadingMore, error, load, loadMore }
}
