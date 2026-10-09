// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { restoreFixture } from '../test-utils/restoreFixtures'
import { mockWebSocket, resetWsHandlers, wsHandlers } from '../test-utils/sharedMocks'

vi.mock('./useWebSocket', () => mockWebSocket())

vi.mock('../api/archives', () => ({
  getRestore: vi.fn(),
}))

import { getRestore } from '../api/archives'
import {
  RESTORE_POLL_MS,
  isRestoreFinished,
  useRestoreTracker,
  type RestoreTracker,
} from './useRestoreTracker'

/** The tracker subscribes during setup, so it needs a mounted component. */
function mountTracker(): { tracker: RestoreTracker; unmount: () => void } {
  let tracker: RestoreTracker | null = null
  const wrapper = mount(
    defineComponent({
      setup() {
        tracker = useRestoreTracker()
        return () => null
      },
    }),
  )
  return { tracker: tracker!, unmount: () => wrapper.unmount() }
}

describe('useRestoreTracker', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.mocked(getRestore).mockReset()
    resetWsHandlers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('tells finished states from the ones a restore still leaves', () => {
    expect(isRestoreFinished('queued')).toBe(false)
    expect(isRestoreFinished('dispatched')).toBe(false)
    expect(isRestoreFinished('running')).toBe(false)
    expect(isRestoreFinished('succeeded')).toBe(true)
    expect(isRestoreFinished('failed')).toBe(true)
    expect(isRestoreFinished('cancelled')).toBe(true)
  })

  it('resolves at once for a restore that already ended', async () => {
    const { tracker } = mountTracker()
    const done = restoreFixture()

    await expect(tracker.follow(done)).resolves.toEqual(done)
    expect(getRestore).not.toHaveBeenCalled()
  })

  it('re-reads the restore on a live update and resolves once it ends', async () => {
    const { tracker } = mountTracker()
    const followed = tracker.follow(restoreFixture({ id: 4, status: 'dispatched' }))
    vi.mocked(getRestore).mockResolvedValueOnce(restoreFixture({ id: 4, status: 'running' }))

    wsHandlers.RestoreUpdated({ restore_id: 4, status: 'running' })
    await flushPromises()
    expect(tracker.restore.value?.status).toBe('running')

    vi.mocked(getRestore).mockResolvedValueOnce(restoreFixture({ id: 4 }))
    wsHandlers.RestoreUpdated({ restore_id: 4, status: 'succeeded' })

    await expect(followed).resolves.toMatchObject({ id: 4, status: 'succeeded' })
    expect(getRestore).toHaveBeenCalledWith(4)
  })

  it('ignores updates for restores it does not follow', async () => {
    const { tracker } = mountTracker()
    void tracker.follow(restoreFixture({ id: 4, status: 'queued' }))

    wsHandlers.RestoreUpdated({ restore_id: 5, status: 'succeeded' })
    await flushPromises()

    expect(getRestore).not.toHaveBeenCalled()
  })

  it('polls when no live update arrives', async () => {
    const { tracker } = mountTracker()
    const followed = tracker.follow(restoreFixture({ id: 4, status: 'running' }))
    vi.mocked(getRestore).mockResolvedValue(
      restoreFixture({ id: 4, status: 'failed', error_message: 'disk full' }),
    )

    await vi.advanceTimersByTimeAsync(RESTORE_POLL_MS)

    await expect(followed).resolves.toMatchObject({ status: 'failed', error_message: 'disk full' })
    vi.mocked(getRestore).mockClear()
    await vi.advanceTimersByTimeAsync(RESTORE_POLL_MS * 2)
    expect(getRestore).not.toHaveBeenCalled()
  })

  it('follows several restores at once', async () => {
    const { tracker } = mountTracker()
    const first = tracker.follow(restoreFixture({ id: 1, status: 'running' }))
    const second = tracker.follow(restoreFixture({ id: 2, status: 'running' }))

    vi.mocked(getRestore).mockResolvedValueOnce(restoreFixture({ id: 2 }))
    wsHandlers.RestoreUpdated({ restore_id: 2, status: 'succeeded' })
    await expect(second).resolves.toMatchObject({ id: 2 })

    tracker.update(restoreFixture({ id: 1, status: 'cancelled' }))
    await expect(first).resolves.toMatchObject({ id: 1, status: 'cancelled' })
  })

  it('stops polling once the component goes away', async () => {
    const { tracker, unmount } = mountTracker()
    void tracker.follow(restoreFixture({ id: 4, status: 'queued' }))

    unmount()
    await vi.advanceTimersByTimeAsync(RESTORE_POLL_MS * 3)

    expect(getRestore).not.toHaveBeenCalled()
  })

  it('keeps following after a failed re-read', async () => {
    const { tracker } = mountTracker()
    const followed = tracker.follow(restoreFixture({ id: 4, status: 'running' }))
    vi.mocked(getRestore).mockRejectedValueOnce(new Error('network down'))
    await vi.advanceTimersByTimeAsync(RESTORE_POLL_MS)

    vi.mocked(getRestore).mockResolvedValueOnce(restoreFixture({ id: 4 }))
    await vi.advanceTimersByTimeAsync(RESTORE_POLL_MS)

    await expect(followed).resolves.toMatchObject({ status: 'succeeded' })
  })
})
