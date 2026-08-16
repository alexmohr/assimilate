// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ComponentPublicInstance } from 'vue'
import type { VueWrapper } from '@vue/test-utils'

/**
 * The registry of wrappers `renderWithPlugins` has mounted, kept in its own
 * module so `setup.ts` can reach it without importing the test-utils barrel.
 *
 * That indirection is load-bearing: the barrel pulls in the router, the store
 * and `api/client`, and evaluating those from a vitest setup file runs them
 * before a test file's `vi.mock` calls are installed - which left
 * `api/client.test.ts` inspecting interceptors that had already been
 * registered on the real module. Type-only imports here keep this module free
 * of runtime dependencies entirely.
 */
const rendered: VueWrapper<ComponentPublicInstance>[] = []

export function trackRendered(wrapper: VueWrapper<ComponentPublicInstance>): void {
  rendered.push(wrapper)
}

/**
 * Unmounts everything `renderWithPlugins` mounted, and forgets it.
 *
 * Clearing `document.body` removes a test's markup but leaves the component
 * running, so anything it holds outside the DOM - a pending `setTimeout`, a
 * subscription - survived into the next test. `RepoCreateDialog`'s 300ms
 * path-autocomplete debounce did exactly that, firing an `/ssh/list-dir`
 * request from a later case in the same file and breaking that case's
 * `toHaveBeenLastCalledWith`. It only surfaced when the machine was loaded
 * enough for the timer to land inside the next test rather than between the
 * two, which made a leak look like flakiness.
 *
 * Only wrappers from `renderWithPlugins` are tracked. Files that call `mount`
 * directly manage their own lifetime, and several of them mount components
 * whose teardown is part of what they assert.
 */
export function unmountRendered(): void {
  while (rendered.length > 0) {
    rendered.pop()?.unmount()
  }
}
