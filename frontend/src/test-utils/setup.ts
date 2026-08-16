// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach } from 'vitest'

/**
 * `renderWithPlugins` attaches to `document.body` so that tests can reach modal
 * content either through the wrapper or through a `document` query. Attached
 * wrappers are not removed automatically, so clear the document between tests
 * to stop one test's markup leaking into the next one's assertions.
 */
afterEach(() => {
  document.body.innerHTML = ''
  document.documentElement.style.overflow = ''
})
