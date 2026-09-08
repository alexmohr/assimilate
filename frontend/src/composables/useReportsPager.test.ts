// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { useReportsPager, REPORTS_PAGE_SIZE, type ReportsPage } from './useReportsPager'
import type { ReportRow } from '../types/report'

function report(id: number): ReportRow {
  return {
    id,
    agent_id: 5,
    repo_id: 10,
    repo_name: 'server-daily',
    schedule_id: null,
    schedule_name: null,
    status: 'success',
    started_at: '2026-06-01T09:00:00Z',
    finished_at: '2026-06-01T10:00:00Z',
    duration_secs: 221,
    original_size: 1024,
    deduplicated_size: 256,
    files_processed: 128,
    error_message: null,
    warnings: [],
    archive_name: null,
    run_id: null,
  } as unknown as ReportRow
}

function page(count: number, total: number): ReportsPage {
  return { reports: Array.from({ length: count }, (_, i) => report(i)), total }
}

describe('useReportsPager', () => {
  it('load() replaces the list with a first page and the reported total', async () => {
    const fetchPage = vi.fn().mockResolvedValue(page(REPORTS_PAGE_SIZE, 312))
    const pager = useReportsPager(fetchPage)

    await pager.load()

    expect(fetchPage).toHaveBeenCalledWith(REPORTS_PAGE_SIZE, 0)
    expect(pager.reports.value).toHaveLength(REPORTS_PAGE_SIZE)
    expect(pager.total.value).toBe(312)
    expect(pager.loading.value).toBe(false)
  })

  it('loadMore() appends the next page, offset by what is already loaded', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(REPORTS_PAGE_SIZE, 70))
      .mockResolvedValueOnce(page(20, 70))
    const pager = useReportsPager(fetchPage)

    await pager.load()
    await pager.loadMore()

    expect(fetchPage).toHaveBeenNthCalledWith(2, REPORTS_PAGE_SIZE, REPORTS_PAGE_SIZE)
    expect(pager.reports.value).toHaveLength(REPORTS_PAGE_SIZE + 20)
    expect(pager.total.value).toBe(70)
    expect(pager.loadingMore.value).toBe(false)
  })

  it('surfaces a fetch failure as an error without touching the existing list', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(10, 10))
      .mockRejectedValueOnce(new Error('network down'))
    const pager = useReportsPager(fetchPage)

    await pager.load()
    await pager.loadMore()

    expect(pager.error.value).toBe('network down')
    expect(pager.reports.value).toHaveLength(10)
  })

  // Regression: a page-load fetch that started first (e.g. the initial
  // mount) can still be in flight when something else (a just-clicked
  // action) calls load() again for a fresher snapshot. Without ordering by
  // start time rather than arrival time, the first call's response landing
  // after the second's would silently overwrite the fresher data with a
  // stale one - exactly the case that let a newly-dispatched backup's
  // pending report vanish from a page that had just refetched it.
  it('a slower call started earlier does not clobber a faster call started later', async () => {
    let resolveFirst!: (p: ReportsPage) => void
    let resolveSecond!: (p: ReportsPage) => void
    const fetchPage = vi
      .fn()
      .mockImplementationOnce(() => new Promise<ReportsPage>((r) => (resolveFirst = r)))
      .mockImplementationOnce(() => new Promise<ReportsPage>((r) => (resolveSecond = r)))
    const pager = useReportsPager(fetchPage)

    const firstLoad = pager.load()
    const secondLoad = pager.load()

    // The second call's response arrives first...
    resolveSecond(page(1, 1))
    await secondLoad
    expect(pager.reports.value).toHaveLength(1)

    // ...then the first (stale) call's response arrives late. It must not
    // undo the second call's result.
    resolveFirst(page(5, 5))
    await firstLoad
    expect(pager.reports.value).toHaveLength(1)
    expect(pager.total.value).toBe(1)
  })

  // Regression: load() bumping loadToken while a loadMore() is still in
  // flight made that loadMore()'s own `finally` block's token check fail,
  // which left `loadingMore` stuck true forever - permanently disabling the
  // Logs tab's "Load more" button until the component remounted. Reachable
  // any time a live refresh (e.g. a WebSocket DataChanged event) fires while
  // a user is mid-"Load more".
  it('a load() that preempts an in-flight loadMore() clears loadingMore', async () => {
    let resolveLoadMore!: (p: ReportsPage) => void
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(REPORTS_PAGE_SIZE, 100))
      .mockImplementationOnce(() => new Promise<ReportsPage>((r) => (resolveLoadMore = r)))
      .mockResolvedValueOnce(page(REPORTS_PAGE_SIZE, 100))
    const pager = useReportsPager(fetchPage)

    await pager.load()
    const loadMore = pager.loadMore()
    expect(pager.loadingMore.value).toBe(true)

    await pager.load()
    expect(pager.loadingMore.value).toBe(false)

    resolveLoadMore(page(20, 100))
    await loadMore
    expect(pager.loadingMore.value).toBe(false)
  })

  // Regression: load() is not just the initial fetch - both detail views
  // also run it on every WebSocket DataChanged event (fleet-wide, not
  // scoped to this agent/schedule) to keep the Logs tab live. A flat
  // REPORTS_PAGE_SIZE refetch there silently collapsed a user's "Load more"
  // progress back to page 1 the next time any unrelated backup started or
  // finished, with nothing on screen to explain the row count dropping.
  it('load() re-fetches enough rows to cover what was already loaded, not just a first page', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce(page(REPORTS_PAGE_SIZE, 120))
      .mockResolvedValueOnce(page(20, 120))
      .mockResolvedValueOnce(page(REPORTS_PAGE_SIZE + 20, 120))
    const pager = useReportsPager(fetchPage)

    await pager.load()
    await pager.loadMore()
    expect(pager.reports.value).toHaveLength(REPORTS_PAGE_SIZE + 20)

    // Simulates a live refresh (e.g. DataChanged) firing after the user has
    // paged past the first 50 rows.
    await pager.load()

    expect(fetchPage).toHaveBeenNthCalledWith(3, REPORTS_PAGE_SIZE + 20, 0)
    expect(pager.reports.value).toHaveLength(REPORTS_PAGE_SIZE + 20)
  })
})
