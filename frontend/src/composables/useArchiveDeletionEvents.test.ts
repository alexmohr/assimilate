// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'

const wsHandlers: Record<string, (payload: unknown) => void> = {}

vi.mock('./useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: (type: string, cb: (payload: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
}))

import { useArchiveDeletionEvents } from './useArchiveDeletionEvents'

const REPO_ID = 7

function target() {
  return {
    onArchiveDeleted: vi.fn(),
    onDataChanged: vi.fn(),
    onRepoIdle: vi.fn(),
  }
}

/**
 * The composable subscribes during setup, so it needs a mounted component -
 * `useWebSocket` registers an `onUnmounted` cleanup of its own.
 */
function subscribe(options: {
  target: () => ReturnType<typeof target> | null
  repoIds?: () => readonly number[]
  reload?: () => Promise<unknown>
}) {
  return mount(
    defineComponent({
      setup() {
        useArchiveDeletionEvents({
          target: options.target,
          repoIds: options.repoIds ?? ((): readonly number[] => [REPO_ID]),
          reload: options.reload ?? ((): Promise<unknown> => Promise.resolve()),
        })
        return () => null
      },
    }),
  )
}

describe('useArchiveDeletionEvents', () => {
  beforeEach(() => {
    for (const type of Object.keys(wsHandlers)) delete wsHandlers[type]
  })

  it('names the finished archive on ArchiveDeleted', () => {
    const t = target()
    subscribe({ target: () => t })

    wsHandlers.ArchiveDeleted({ repo_id: REPO_ID, archive_name: 'web-01-2026-03-01' })

    expect(t.onArchiveDeleted).toHaveBeenCalledWith('web-01-2026-03-01', REPO_ID)
  })

  // A screen can hold deletion state for more than the repository it is
  // showing: the schedule Backups tab's scope selector moves between targets
  // while a delete is still running, and the marker for the one left behind
  // survives that move. Filtering to the visible repository would drop the
  // only event that ever releases it.
  it('forwards an event for any repository the screen still holds state for', () => {
    const OTHER = REPO_ID + 1
    const t = target()
    subscribe({ target: () => t, repoIds: () => [REPO_ID, OTHER] })

    wsHandlers.ArchiveDeleted({ repo_id: OTHER, archive_name: 'web-01-2026-03-01' })
    wsHandlers.RepoOpChanged({ repo_id: OTHER, op: null })

    // Named, so the caller clears the copy the event is about rather than
    // whichever one happens to be on screen.
    expect(t.onArchiveDeleted).toHaveBeenCalledWith('web-01-2026-03-01', OTHER)
    expect(t.onRepoIdle).toHaveBeenCalledWith(OTHER)
  })

  it('ignores events for a repository this screen holds no state for', () => {
    const t = target()
    subscribe({ target: () => t })

    wsHandlers.ArchiveDeleted({ repo_id: REPO_ID + 1, archive_name: 'web-01-2026-03-01' })
    wsHandlers.RepoOpChanged({ repo_id: REPO_ID + 1, op: null })

    expect(t.onArchiveDeleted).not.toHaveBeenCalled()
    expect(t.onRepoIdle).not.toHaveBeenCalled()
  })

  it('prunes only after the reload it asked for has resolved', async () => {
    const t = target()
    let resolveReload: () => void = () => {}
    const reload = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveReload = resolve
        }),
    )
    subscribe({ target: () => t, reload })

    wsHandlers.DataChanged({})
    await Promise.resolve()

    expect(reload).toHaveBeenCalled()
    expect(t.onDataChanged).not.toHaveBeenCalled()

    resolveReload()
    await Promise.resolve()
    await Promise.resolve()

    expect(t.onDataChanged).toHaveBeenCalled()
  })

  it('sweeps once the queue holds neither a delete nor its trailing compact', () => {
    const t = target()
    subscribe({ target: () => t })

    // Still deleting, and the compact borg runs straight afterwards: the
    // delete has not concluded either way yet.
    wsHandlers.RepoOpChanged({ repo_id: REPO_ID, op: { kind: 'delete_archive' } })
    wsHandlers.RepoOpChanged({ repo_id: REPO_ID, op: { kind: 'compact_repo' } })
    expect(t.onRepoIdle).not.toHaveBeenCalled()

    wsHandlers.RepoOpChanged({ repo_id: REPO_ID, op: null })
    expect(t.onRepoIdle).toHaveBeenCalledTimes(1)
  })

  it('does nothing when the screen it belongs to is not mounted', async () => {
    const reload = vi.fn(() => Promise.resolve())
    subscribe({ target: () => null, reload })

    wsHandlers.ArchiveDeleted({ repo_id: REPO_ID, archive_name: 'web-01-2026-03-01' })
    wsHandlers.RepoOpChanged({ repo_id: REPO_ID, op: null })
    wsHandlers.DataChanged({})
    await Promise.resolve()

    // The list still refreshes - other tabs render it - there is just nothing
    // holding deletion markers to reconcile.
    expect(reload).toHaveBeenCalled()
  })

  it('swallows a failed reload rather than leaving an unhandled rejection', async () => {
    const t = target()
    subscribe({ target: () => t, reload: () => Promise.reject(new Error('offline')) })

    wsHandlers.DataChanged({})
    await Promise.resolve()
    await Promise.resolve()

    expect(t.onDataChanged).not.toHaveBeenCalled()
  })
})
