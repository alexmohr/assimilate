// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, vi } from 'vitest'
import { unmountRendered } from './rendered'

/**
 * A WebSocket that never connects, and so never closes either. The real
 * `useWebSocket` connects as soon as anything imports it; against the test
 * environment's own WebSocket that connection fails, and the reconnect it
 * schedules can fire after the file's environment is torn down, failing the
 * whole run with "window is not defined". A test that exercises the socket
 * (`useWebSocket.test.ts`) stubs its own.
 */
class InertWebSocket extends EventTarget {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readonly readyState = InertWebSocket.CONNECTING
  constructor(readonly url: string) {
    super()
  }
  send(): void {}
  close(): void {}
}
vi.stubGlobal('WebSocket', InertWebSocket)

/**
 * `renderWithPlugins` attaches to `document.body` so that tests can reach modal
 * content either through the wrapper or through a `document` query. Attached
 * wrappers are not removed automatically, so unmount them and clear the
 * document between tests to stop one test's markup - or its still-running
 * component - leaking into the next one's assertions.
 *
 * Unmounting has to come first: it is what runs each component's
 * `onBeforeUnmount`, and clearing the document out from under a live component
 * would leave it patching nodes that are no longer there.
 *
 * `localStorage` gets the same treatment: vitest gives every test *file* its
 * own environment, but not every test *within* one, so a view that persists
 * its sort/filter/group state (see `usePersistedRef`) would otherwise carry
 * whatever an earlier test in the same file left behind into the next one's
 * initial mount.
 */
afterEach(() => {
  unmountRendered()
  document.body.innerHTML = ''
  document.documentElement.style.overflow = ''
  localStorage.clear()
})
