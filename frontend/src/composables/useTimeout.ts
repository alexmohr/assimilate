// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { getCurrentScope, onScopeDispose } from 'vue'

export interface UseTimeoutReturn {
  /** Runs `callback` after `ms`, replacing whatever was still pending. */
  start: (callback: () => void, ms: number) => void
  /** Cancels the pending callback, if any. */
  clear: () => void
  /** True while a callback is scheduled and has not run yet. */
  isPending: () => boolean
}

/**
 * A single restartable timeout that is cancelled when the calling component
 * (or effect scope) is torn down.
 *
 * A bare `setTimeout` in a view outlives the view: a "Saved" flag reset three
 * seconds after a save, or a search debounce, still fires after the user has
 * navigated away, writing to refs nothing renders any more - or, for a
 * debounce, issuing a request on behalf of a page that no longer exists.
 * Starting again replaces the pending callback, which is exactly what both a
 * debounce and a flash message want.
 */
export function useTimeout(): UseTimeoutReturn {
  let timer: ReturnType<typeof setTimeout> | null = null

  function clear(): void {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
  }

  function start(callback: () => void, ms: number): void {
    clear()
    timer = setTimeout(() => {
      timer = null
      callback()
    }, ms)
  }

  if (getCurrentScope()) onScopeDispose(clear)

  return { start, clear, isPending: () => timer !== null }
}
