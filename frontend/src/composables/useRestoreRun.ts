// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, type Ref } from 'vue'
import { getRestoreRun } from '../api/restores'
import type { RestoreRun } from '../types/generated'
import { isRestoreFinished } from '../utils/restoreRun'
import { logger } from '../utils/logger'
import { useWebSocket } from './useWebSocket'

interface UseRestoreRunReturn {
  /** The restore being followed, as it now stands; null before `follow`. */
  run: Ref<RestoreRun | null>
  /** Starts following `started`, the restore the API just recorded. */
  follow: (started: RestoreRun) => void
  /** Follows `started` and resolves with it once it is over. */
  untilFinished: (started: RestoreRun) => Promise<RestoreRun>
}

/**
 * Follows one restore onto an agent as it waits for the agent, runs and
 * ends, from the server's `RestoreRunChanged` pushes. A restore can finish
 * before the request that started it returns, so following re-reads it once
 * after it starts listening, and again whenever the UI WebSocket reconnects,
 * since pushes sent while it was down are lost.
 *
 * Must be called during a component's setup: the push listener is removed
 * when the component unmounts.
 */
export function useRestoreRun(): UseRestoreRunReturn {
  const run = ref<RestoreRun | null>(null)
  let onFinished: ((finished: RestoreRun) => void) | null = null

  const { onMessage, status } = useWebSocket()

  /** Takes `next` unless it would reopen a restore already known to be over. */
  function update(next: RestoreRun): void {
    const current = run.value
    if (current === null || current.id !== next.id) return
    if (isRestoreFinished(current.status) && !isRestoreFinished(next.status)) return
    run.value = next
    if (isRestoreFinished(next.status) && onFinished !== null) {
      const resolve = onFinished
      onFinished = null
      resolve(next)
    }
  }

  async function refresh(): Promise<void> {
    const current = run.value
    if (current === null || isRestoreFinished(current.status)) return
    try {
      update(await getRestoreRun(current.id))
    } catch (e: unknown) {
      logger.warn('could not refresh the restore', e)
    }
  }

  onMessage('RestoreRunChanged', (payload) => update(payload.run))

  watch(
    () => status.value,
    (now) => {
      if (now === 'connected') refresh().catch(logger.error)
    },
  )

  function follow(started: RestoreRun): void {
    run.value = started
    refresh().catch(logger.error)
  }

  function untilFinished(started: RestoreRun): Promise<RestoreRun> {
    return new Promise((resolve) => {
      onFinished = resolve
      follow(started)
      if (isRestoreFinished(started.status)) update(started)
    })
  }

  return { run, follow, untilFinished }
}
