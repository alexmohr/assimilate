// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach } from 'vitest'
import { unmountRendered } from './rendered'

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
 */
afterEach(() => {
  unmountRendered()
  document.body.innerHTML = ''
  document.documentElement.style.overflow = ''
})
