// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const toast = { success: vi.fn(), error: vi.fn(), info: vi.fn() }
vi.mock('./useToast', () => ({ useToast: () => toast }))
vi.mock('./useWebSocket', () => import('../test-utils/wsMock'))
vi.mock('../api/restores', () => ({ getRestoreRun: vi.fn() }))

import { getRestoreRun } from '../api/restores'
import { makeRestoreRun } from '../test-utils/restoreRun'
import { pushWs, resetWsMock, setWsStatus } from '../test-utils/wsMock'
import { useRestoreToasts } from './useRestoreToasts'

function push(run = makeRestoreRun()): void {
  pushWs('RestoreRunChanged', { run })
}

describe('useRestoreToasts', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    resetWsMock()
    vi.mocked(getRestoreRun).mockResolvedValue(makeRestoreRun())
  })

  it('says a restore started, and then how it ended', async () => {
    const { track } = useRestoreToasts()
    track(makeRestoreRun())
    await flushPromises()
    expect(toast.info).toHaveBeenCalledWith('Restoring etc/hosts onto web-01...')

    push(makeRestoreRun({ status: 'success' }))
    push(makeRestoreRun({ status: 'success' }))

    // Once: the restore is no longer tracked after it ended.
    expect(toast.success).toHaveBeenCalledTimes(1)
    expect(toast.success).toHaveBeenCalledWith('Restored etc/hosts onto web-01.')
  })

  it('says a restore waits for an offline agent', () => {
    const { track } = useRestoreToasts()

    track(makeRestoreRun({ status: 'pending' }))

    expect(toast.info).toHaveBeenCalledWith(
      'web-01 is offline. Restoring etc/hosts once it connects.',
    )
  })

  it('reports a failed restore with its reason', async () => {
    const { track } = useRestoreToasts()
    track(makeRestoreRun({ paths: [] }))
    await flushPromises()

    push(makeRestoreRun({ paths: [], status: 'failed', error_message: 'disk full' }))
    push(makeRestoreRun({ id: 'untracked', status: 'failed' }))

    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(toast.error).toHaveBeenCalledWith(
      'Restoring the whole archive onto web-01 failed: disk full',
    )
  })

  it('reports a cancelled restore', async () => {
    const { track } = useRestoreToasts()
    track(makeRestoreRun({ status: 'pending' }))
    await flushPromises()

    push(makeRestoreRun({ status: 'cancelled' }))

    expect(toast.info).toHaveBeenLastCalledWith('Restoring etc/hosts onto web-01 was cancelled.')
  })

  it('reports a restore that ended before the request returned', async () => {
    vi.mocked(getRestoreRun).mockResolvedValue(makeRestoreRun({ status: 'success' }))
    const { track } = useRestoreToasts()

    track(makeRestoreRun())
    await flushPromises()

    expect(toast.success).toHaveBeenCalledWith('Restored etc/hosts onto web-01.')
  })

  it('catches up on tracked restores when the UI WebSocket reconnects', async () => {
    const { track } = useRestoreToasts()
    track(makeRestoreRun())
    await flushPromises()
    setWsStatus('reconnecting')
    await flushPromises()
    vi.mocked(getRestoreRun).mockRejectedValueOnce(new Error('offline'))
    vi.mocked(getRestoreRun).mockResolvedValue(makeRestoreRun({ status: 'failed' }))

    setWsStatus('connected')
    await flushPromises()
    expect(toast.error).not.toHaveBeenCalled()

    setWsStatus('reconnecting')
    await flushPromises()
    setWsStatus('connected')
    await flushPromises()
    expect(toast.error).toHaveBeenCalledWith(
      'Restoring etc/hosts onto web-01 failed: unknown error',
    )
  })
})
