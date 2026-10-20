// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mockApiClientRw, mockToast, resetToastSpies, toastSpies } from '../test-utils/sharedMocks'
import { useReportRetry } from './useReportRetry'
import type { ReportRow } from '../types/report'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('./useToast', () => mockToast())

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
    resetToastSpies()
    mockPost.mockResolvedValue({ data: {} } as never)
  })

  it('re-runs only the failed copy: its host and its repository', async () => {
    const { retry } = useReportRetry(vi.fn())
    await retry(report())
    expect(mockPost).toHaveBeenCalledWith('/schedules/7/run', {
      agent_ids: [3],
      repo_ids: [8],
    })
    expect(toastSpies.success).toHaveBeenCalledWith('Retry queued for hetzner-global on dragon.')
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

  function holdPost(): () => void {
    let release = (): void => {}
    mockPost.mockReturnValue(
      new Promise((resolve) => {
        release = (): void => resolve({ data: {} } as never)
      }) as never,
    )
    return () => release()
  }

  it('marks the row while the request is in flight and ignores a second press on it', async () => {
    const release = holdPost()
    const { retry, retryingReportIds } = useReportRetry(vi.fn())
    const first = retry(report())
    expect([...retryingReportIds.value]).toEqual([42])
    await retry(report())
    expect(mockPost).toHaveBeenCalledOnce()
    release()
    await first
    expect(retryingReportIds.value.size).toBe(0)
  })

  it('lets another failed row retry while the first is still being sent', async () => {
    // A run that failed on two repositories leaves two failed rows; the
    // second click must reach the server, not vanish behind the first.
    const release = holdPost()
    const { retry, retryingReportIds } = useReportRetry(vi.fn())
    const first = retry(report())
    const second = retry(report({ id: 43, repo_id: 9 }))
    expect([...retryingReportIds.value].sort()).toEqual([42, 43])
    expect(mockPost).toHaveBeenCalledTimes(2)
    expect(mockPost).toHaveBeenLastCalledWith('/schedules/7/run', {
      agent_ids: [3],
      repo_ids: [9],
    })
    release()
    await Promise.all([first, second])
    expect(retryingReportIds.value.size).toBe(0)
  })

  it('reports a refused retry and leaves the log alone', async () => {
    mockPost.mockRejectedValue(new Error('Forbidden'))
    const onQueued = vi.fn()
    const { retry, retryingReportIds } = useReportRetry(onQueued)
    await retry(report())
    expect(toastSpies.error).toHaveBeenCalledWith('Forbidden')
    expect(onQueued).not.toHaveBeenCalled()
    expect(retryingReportIds.value.size).toBe(0)
  })
})
