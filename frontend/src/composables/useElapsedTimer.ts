// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, onUnmounted, type Ref } from 'vue'

/**
 * A reactive clock that ticks once a second while `active` is true, and
 * stops automatically the moment it isn't - so callers get a live "now" to
 * derive elapsed/remaining durations from without each maintaining their own
 * start/stop interval bookkeeping.
 */
export function useElapsedClock(active: Ref<boolean>): { now: Ref<number> } {
  const now = ref(Date.now())
  let timer: ReturnType<typeof setInterval> | null = null

  function ensureTimer(): void {
    if (timer !== null) return
    timer = setInterval(() => {
      now.value = Date.now()
    }, 1000)
  }

  function stopTimer(): void {
    if (timer !== null) {
      clearInterval(timer)
      timer = null
    }
  }

  watch(
    active,
    (isActive) => {
      if (isActive) {
        ensureTimer()
      } else {
        stopTimer()
      }
    },
    { immediate: true },
  )

  onUnmounted(stopTimer)

  return { now }
}
