// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { watch } from 'vue'
import { getRestoreRun } from '../api/restores'
import type { RestoreRun } from '../types/generated'
import { isRestoreFinished, restoreScope } from '../utils/restoreRun'
import { logger } from '../utils/logger'
import { useToast } from './useToast'
import { useWebSocket } from './useWebSocket'

/**
 * Reports restores onto agents with toasts: one when a restore starts, and
 * one when it ends, from the server's `RestoreRunChanged` pushes. Any number
 * of restores can be tracked at once.
 *
 * Must be called during a component's setup; restores still running when
 * the component unmounts are no longer reported (the Activity Log lists
 * them).
 */
export function useRestoreToasts(): { track: (started: RestoreRun) => void } {
  const tracked = new Set<string>()
  const toast = useToast()
  const { onMessage, status } = useWebSocket()

  function settle(run: RestoreRun): void {
    if (!tracked.has(run.id) || !isRestoreFinished(run.status)) return
    tracked.delete(run.id)
    const what = `${restoreScope(run)} onto ${run.hostname}`
    switch (run.status) {
      case 'success':
        toast.success(`Restored ${what}.`)
        break
      case 'failed':
        toast.error(`Restoring ${what} failed: ${run.error_message ?? 'unknown error'}`)
        break
      case 'cancelled':
      case 'pending':
      case 'running':
        toast.info(`Restoring ${what} was cancelled.`)
        break
    }
  }

  /** A restore can end before the request that started it returns, and
   * pushes sent while the UI WebSocket was down are lost: re-read. */
  async function refresh(id: string): Promise<void> {
    try {
      settle(await getRestoreRun(id))
    } catch (e: unknown) {
      logger.warn('could not refresh the restore', e)
    }
  }

  onMessage('RestoreRunChanged', (payload) => settle(payload.run))

  watch(
    () => status.value,
    (now) => {
      if (now === 'connected') {
        for (const id of tracked) refresh(id).catch(logger.error)
      }
    },
  )

  function track(started: RestoreRun): void {
    tracked.add(started.id)
    const what = `${restoreScope(started)} onto ${started.hostname}`
    if (started.status === 'pending') {
      toast.info(
        `${started.hostname} is offline. Restoring ${restoreScope(started)} once it connects.`,
      )
    } else if (!isRestoreFinished(started.status)) {
      toast.info(`Restoring ${what}...`)
    }
    settle(started)
    refresh(started.id).catch(logger.error)
  }

  return { track }
}
