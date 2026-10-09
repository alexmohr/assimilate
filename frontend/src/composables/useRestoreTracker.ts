// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { onScopeDispose, ref, type Ref } from 'vue'
import { getRestore } from '../api/archives'
import { logger } from '../utils/logger'
import { useWebSocket } from './useWebSocket'
import type { RestoreResponse, RestoreStatus } from '../types/generated'

/** How often a followed restore is re-read when no live update arrives. */
export const RESTORE_POLL_MS = 5_000

const FINISHED: ReadonlySet<RestoreStatus> = new Set<RestoreStatus>([
  'succeeded',
  'failed',
  'cancelled',
])

export function isRestoreFinished(status: RestoreStatus): boolean {
  return FINISHED.has(status)
}

export interface RestoreTracker {
  /** The restore followed most recently, as last read. */
  restore: Ref<RestoreResponse | null>
  /**
   * Follows `started` until it ends and resolves with how it ended. The
   * component going away abandons every restore it follows: their promises
   * never resolve, and the restores themselves carry on on the agent.
   */
  follow: (started: RestoreResponse) => Promise<RestoreResponse>
  /** Records a restore read elsewhere - the answer to a cancel, say. */
  update: (latest: RestoreResponse) => void
}

/**
 * Follows restores onto agents from the moment each is accepted to its end.
 *
 * A restore runs on the agent long after the request that started it
 * returned, and one for an offline agent waits until it reconnects. The live
 * `RestoreUpdated` event carries only the id and state, so each one triggers
 * a re-read of that restore; a slow poll covers a WebSocket that was
 * reconnecting when the update went out.
 */
export function useRestoreTracker(): RestoreTracker {
  const restore = ref<RestoreResponse | null>(null)
  const pending = new Map<number, (finished: RestoreResponse) => void>()
  let pollTimer: ReturnType<typeof setInterval> | null = null

  function stopPolling(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  function update(latest: RestoreResponse): void {
    if (restore.value?.id === latest.id) restore.value = latest
    if (!isRestoreFinished(latest.status)) return
    pending.get(latest.id)?.(latest)
    pending.delete(latest.id)
    if (pending.size === 0) stopPolling()
  }

  async function refresh(restoreId: number): Promise<void> {
    try {
      update(await getRestore(restoreId))
    } catch (e: unknown) {
      logger.warn('failed to re-read a restore', e)
    }
  }

  function follow(started: RestoreResponse): Promise<RestoreResponse> {
    restore.value = started
    if (isRestoreFinished(started.status)) return Promise.resolve(started)
    return new Promise<RestoreResponse>((resolve) => {
      pending.set(started.id, resolve)
      pollTimer ??= setInterval(() => {
        for (const restoreId of pending.keys()) void refresh(restoreId)
      }, RESTORE_POLL_MS)
    })
  }

  const { onMessage } = useWebSocket()
  onMessage('RestoreUpdated', (payload) => {
    if (pending.has(payload.restore_id)) void refresh(payload.restore_id)
  })

  onScopeDispose(() => {
    stopPolling()
    pending.clear()
  })

  return { restore, follow, update }
}
