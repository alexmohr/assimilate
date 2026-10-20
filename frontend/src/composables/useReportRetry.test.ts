// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useReportRetry } from './useReportRetry'
import type { ReportRow } from '../types/report'

const toastSuccess = vi.fn()
const toastError = vi.fn()

vi.mock('./useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

vi.mock('../api/client', () => ({ apiClient: { post: vi.fn() } }))

import { apiClient } from '../api/client'
const mockPost = vi.mocked(apiClient.post)

function report(overrides: Partial<ReportRow> = {}): ReportRow {
  return {
    id: 42,
    agent_id: 3,
    repo_id: 8,
    schedule_id: 7,
    status: 'failed',
    hostname: 'dragon',
    repo_name: 'hetzner-global',
    ...overrides,
  } as ReportRow
}

describe('useReportRetry', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockPost.mockResolvedValue({ data: {} } as never)
  })

  it('re-runs only the failed copy: its host and its repository', async () => {
    const { retry } = useReportRetry(vi.fn())
    await retry(report())
    expect(mockPost).toHaveBeenCalledWith('/schedules/7/run', {
      agent_ids: [3],
      repo_ids: [8],
    })
    expect(toastSuccess).toHaveBeenCalledWith('Retry queued for hetzner-global on dragon.')
  })

  it('reloads the log once the retry is queued', async () => {
    const onQueued = vi.fn()
    const { retry } = useReportRetry(onQueued)
    await retry(report())
    expect(onQueued).toHaveBeenCalledOnce()
  })

  it('does nothing for a run that has no schedule to re-run', async () => {
    const { retry } = useReportRetry(vi.fn())
    await retry(report({ schedule_id: null }))
    expect(mockPost).not.toHaveBeenCalled()
  })

  it('marks the row while the request is in flight and ignores a second press', async () => {
    let release = (): void => {}
    mockPost.mockReturnValue(
      new Promise((resolve) => {
        release = (): void => resolve({ data: {} } as never)
      }) as never,
    )
    const { retry, retryingReportId } = useReportRetry(vi.fn())
    const first = retry(report())
    expect(retryingReportId.value).toBe(42)
    await retry(report({ id: 43 }))
    expect(mockPost).toHaveBeenCalledOnce()
    release()
    await first
    expect(retryingReportId.value).toBeNull()
  })

  it('reports a refused retry and leaves the log alone', async () => {
    mockPost.mockRejectedValue(new Error('Forbidden'))
    const onQueued = vi.fn()
    const { retry, retryingReportId } = useReportRetry(onQueued)
    await retry(report())
    expect(toastError).toHaveBeenCalledWith('Forbidden')
    expect(onQueued).not.toHaveBeenCalled()
    expect(retryingReportId.value).toBeNull()
  })
})
