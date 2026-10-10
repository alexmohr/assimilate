// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { makeRestoreRun } from '../test-utils/restoreRun'

import { pushWs, resetWsMock, setWsStatus } from '../test-utils/wsMock'
vi.mock('./useWebSocket', () => import('../test-utils/wsMock'))
vi.mock('../api/restores', () => ({ getRestoreRun: vi.fn() }))

import { getRestoreRun } from '../api/restores'
import { useRestoreRun } from './useRestoreRun'

function push(run = makeRestoreRun()): void {
  pushWs('RestoreRunChanged', { run })
}

describe('useRestoreRun', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    resetWsMock()
    // By default the server still has the restore running.
    vi.mocked(getRestoreRun).mockResolvedValue(makeRestoreRun())
  })

  it('follows the restore through its pushes', async () => {
    const { run, follow } = useRestoreRun()
    follow(makeRestoreRun({ status: 'pending' }))
    await flushPromises()

    push(makeRestoreRun({ status: 'success', files_restored: 2 }))

    expect(run.value?.status).toBe('success')
    expect(run.value?.files_restored).toBe(2)
  })

  it('ignores pushes about another restore', async () => {
    const { run, follow } = useRestoreRun()
    follow(makeRestoreRun())
    await flushPromises()

    push(makeRestoreRun({ id: 'another', status: 'failed' }))

    expect(run.value?.status).toBe('running')
  })

  it('never reopens a restore known to be over', async () => {
    const { run, follow } = useRestoreRun()
    follow(makeRestoreRun())
    await flushPromises()

    push(makeRestoreRun({ status: 'success' }))
    push(makeRestoreRun({ status: 'running' }))

    expect(run.value?.status).toBe('success')
  })

  it('re-reads the restore, which may have ended before the request returned', async () => {
    vi.mocked(getRestoreRun).mockResolvedValue(makeRestoreRun({ status: 'failed' }))
    const { run, follow } = useRestoreRun()

    follow(makeRestoreRun())
    await flushPromises()

    expect(getRestoreRun).toHaveBeenCalledWith(makeRestoreRun().id)
    expect(run.value?.status).toBe('failed')
  })

  it('re-reads the restore when the UI WebSocket reconnects', async () => {
    const { run, follow } = useRestoreRun()
    follow(makeRestoreRun())
    await flushPromises()
    setWsStatus('reconnecting')
    await flushPromises()
    vi.mocked(getRestoreRun).mockResolvedValue(makeRestoreRun({ status: 'success' }))

    setWsStatus('connected')
    await flushPromises()

    expect(run.value?.status).toBe('success')
  })

  it('keeps following when a re-read fails', async () => {
    vi.mocked(getRestoreRun).mockRejectedValue(new Error('offline'))
    const { run, follow } = useRestoreRun()

    follow(makeRestoreRun())
    await flushPromises()
    push(makeRestoreRun({ status: 'success' }))

    expect(run.value?.status).toBe('success')
  })

  it('resolves untilFinished once the restore is over', async () => {
    const { untilFinished } = useRestoreRun()
    let finished: string | null = null

    untilFinished(makeRestoreRun()).then((r) => (finished = r.status))
    await flushPromises()
    expect(finished).toBeNull()

    push(makeRestoreRun({ status: 'cancelled' }))
    await flushPromises()
    expect(finished).toBe('cancelled')
  })

  it('resolves untilFinished at once for a restore already over', async () => {
    const { untilFinished } = useRestoreRun()

    const finished = await untilFinished(makeRestoreRun({ status: 'success' }))

    expect(finished.status).toBe('success')
    // Nothing more can change, so there is nothing to re-read.
    expect(getRestoreRun).not.toHaveBeenCalled()
  })
})
