// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'

/**
 * A stand-in for `useWebSocket` that records the listeners a component
 * registers, so a test can push server events with `pushWs`. Use it as
 *
 *   vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))
 *
 * and call `resetWsMock()` in `beforeEach`.
 */

const handlers = new Map<string, (payload: unknown) => void>()
// Replaced on reset rather than set back: watchers from an earlier test stay
// on the ref they were given and must not fire for the next one.
let status: Ref<string> = ref('connected')

export function useWebSocket(): {
  onMessage: (type: string, cb: (payload: unknown) => void) => void
  status: Ref<string>
} {
  return {
    onMessage: (type, cb) => {
      handlers.set(type, cb)
    },
    status,
  }
}

/** Delivers `payload` to the listener for `type`, as a server push would. */
export function pushWs(type: string, payload: unknown): void {
  const handler = handlers.get(type)
  if (!handler) throw new Error(`nothing listens for ${type}`)
  handler(payload)
}

/** Sets the connection status, as a drop or reconnect would. */
export function setWsStatus(next: string): void {
  status.value = next
}

export function resetWsMock(): void {
  handlers.clear()
  status = ref('connected')
}
