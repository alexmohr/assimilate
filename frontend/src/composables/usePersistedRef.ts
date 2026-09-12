// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, type Ref } from 'vue'
import { readStorage, writeStorage } from '../utils/storage'

/**
 * A ref that starts from localStorage and writes every change straight back
 * to it - for a view's own toolbar state (how its list is grouped, sorted or
 * filtered), the kind of choice a person makes once and expects to stay made,
 * as opposed to a search box's free text, which is expected to start empty.
 *
 * `isValid` is a type predicate rather than a plain boolean check, so the
 * restored value narrows to `T` without a cast - the frontend's equivalent of
 * Rust's `TryFrom` exemption from `no-string-literal-control-flow`. A value
 * an older or newer build no longer supports (a removed enum member, or
 * simply `null`/corrupted storage) falls back to `initial` instead of
 * poisoning the ref with a value the caller never asked for.
 *
 * `override`, when it validates, wins over both storage and `initial` - a
 * query-string parameter such as `?filter=overdue` behaves like an explicit
 * choice made just now, not one silently masked by whatever was last
 * persisted. The override is itself written back, so it becomes the new
 * persisted value the next time this view loads without one.
 */
export function usePersistedRef<T extends string>(
  key: string,
  initial: T,
  isValid: (value: string) => value is T,
  override?: string | null,
): Ref<T> {
  let start = initial
  if (override != null && isValid(override)) {
    start = override
  } else {
    const stored = readStorage(key)
    if (stored !== undefined && isValid(stored)) {
      start = stored
    }
  }

  const value = ref(start) as Ref<T>
  watch(value, (next) => writeStorage(key, next))
  return value
}

/**
 * The boolean-flag counterpart, for a toggle like "show hidden agents" that
 * has no enum to validate against - just a literal "true"/"false" string in
 * storage, the same shape `stores/ui.ts` already persists the sidebar's
 * collapsed state in.
 */
export function usePersistedBoolean(key: string, initial: boolean): Ref<boolean> {
  const stored = readStorage(key)
  // eslint-disable-next-line local/no-string-literal-control-flow -- boolean flag persisted as the literal string "true"/"false" in localStorage, not domain state
  const value = ref(stored !== undefined ? stored === 'true' : initial)
  watch(value, (next) => writeStorage(key, String(next)))
  return value
}
