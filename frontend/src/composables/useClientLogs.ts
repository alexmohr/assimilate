// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { getCurrentScope, onScopeDispose, shallowRef, type ShallowRef } from 'vue'
import { clientLogBuffer, type ClientLogBuffer, type ClientLogEntry } from '../utils/clientLog'

interface UseClientLogsReturn {
  /** Newest first, matching the Server Logs tab. */
  entries: Readonly<ShallowRef<readonly ClientLogEntry[]>>
  clear: () => void
  /** Removes the buffer listener. Runs on its own when the scope ends. */
  stop: () => void
}

/**
 * A live, newest-first view of the client log buffer. The buffer listener
 * is removed when the calling component unmounts, so an Activity page that
 * is opened and left repeatedly does not pile up subscriptions.
 */
export function useClientLogs(buffer: ClientLogBuffer = clientLogBuffer): UseClientLogsReturn {
  const snapshot = (): readonly ClientLogEntry[] => buffer.entries().reverse()
  const entries = shallowRef<readonly ClientLogEntry[]>(snapshot())
  const stop = buffer.subscribe(() => {
    entries.value = snapshot()
  })
  if (getCurrentScope()) onScopeDispose(stop)

  return {
    entries,
    clear: (): void => buffer.clear(),
    stop,
  }
}
