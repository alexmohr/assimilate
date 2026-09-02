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
})
