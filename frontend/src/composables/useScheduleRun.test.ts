// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useScheduleRun } from './useScheduleRun'
import type { ScheduleRow, ScheduleType } from '../types/schedule'

const toastSuccess = vi.fn()
const toastError = vi.fn()

vi.mock('./useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

vi.mock('../api/client', () => ({ apiClient: { post: vi.fn() } }))

import { apiClient } from '../api/client'
const mockPost = vi.mocked(apiClient.post)

function label(type: ScheduleType): string {
  switch (type) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity check'
    case 'verify':
      return 'Verify'
  }
}

function schedule(overrides: Partial<ScheduleRow> = {}): ScheduleRow {
  return { id: 7, schedule_type: 'backup', ...overrides } as ScheduleRow
}

describe('useScheduleRun', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockPost.mockResolvedValue({ data: {} } as never)
  })

  it('posts the run request for the given schedule', async () => {
    const { runNow } = useScheduleRun(label)
    await runNow(schedule())
    expect(mockPost).toHaveBeenCalledWith('/schedules/7/run', {})
  })

  it('names the kind of run in the toast, not just "started"', async () => {
    const { runNow } = useScheduleRun(label)
    await runNow(schedule({ schedule_type: 'check' }))
    expect(toastSuccess).toHaveBeenCalledWith('Integrity check started.')
  })

  it('stays silent for a caller that does not announce starts', async () => {
    const { runNow } = useScheduleRun(null)
    await runNow(schedule())
    expect(mockPost).toHaveBeenCalledWith('/schedules/7/run', {})
    expect(toastSuccess).not.toHaveBeenCalled()
  })

  it('scopes the run to the caller-supplied body', async () => {
    // The agent page pins the run to the host whose page it is, so pressing
    // "Run now" there must not fire the schedule's other targets too.
    const { runNow } = useScheduleRun(null, { body: (s) => ({ hostname: `host-${s.id}` }) })
    await runNow(schedule({ id: 9 }))
    expect(mockPost).toHaveBeenCalledWith('/schedules/9/run', { hostname: 'host-9' })
  })

  it('marks only the running schedule while the request is in flight', async () => {
    let release = (): void => {}
    mockPost.mockReturnValue(
      new Promise((resolve) => {
        release = (): void => resolve({ data: {} } as never)
      }) as never,
    )

    const { runNow, runNowLoading } = useScheduleRun(label)
    expect(runNowLoading.value).toBe(null)

    const pending = runNow(schedule({ id: 42 }))
    expect(runNowLoading.value).toBe(42)

    release()
    await pending
    expect(runNowLoading.value).toBe(null)
  })

  it('surfaces a failure as a toast and clears the marker', async () => {
    mockPost.mockRejectedValue(new Error('borg is busy'))

    const { runNow, runNowLoading } = useScheduleRun(label)
    await runNow(schedule())

    expect(toastError).toHaveBeenCalledWith(expect.stringContaining('borg is busy'))
    expect(toastSuccess).not.toHaveBeenCalled()
    expect(runNowLoading.value).toBe(null)
  })

  it('routes a failure to the caller-supplied reporter instead of a toast', async () => {
    mockPost.mockRejectedValue(new Error('borg is busy'))
    const onError = vi.fn()

    const { runNow } = useScheduleRun(label, { onError })
    await runNow(schedule())

    expect(onError).toHaveBeenCalledWith(expect.stringContaining('borg is busy'))
    expect(toastError).not.toHaveBeenCalled()
  })

  it('clears a previous failure when the next run starts', async () => {
    const onError = vi.fn()
    const { runNow } = useScheduleRun(label, { onError })

    mockPost.mockRejectedValueOnce(new Error('borg is busy'))
    await runNow(schedule())
    onError.mockClear()

    await runNow(schedule())
    expect(onError).toHaveBeenCalledWith(null)
  })
})
