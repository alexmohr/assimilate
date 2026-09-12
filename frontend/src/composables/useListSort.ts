// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { usePersistedRef } from './usePersistedRef'

export type SortDir = 'asc' | 'desc'

/** Type predicate for `SortDir`, so a persisted sort direction can be restored safely. */
export function isSortDir(value: string): value is SortDir {
  return value === 'asc' || value === 'desc'
}

export interface ListSort<F extends string> {
  /** The column the list is currently ordered by. */
  field: Ref<F>
  direction: Ref<SortDir>
  /**
   * Sort by `field`, flipping the direction when it is already the active one.
   * A fresh column always starts ascending.
   */
  toggle: (field: F) => void
  /** `-1` when descending, so a comparator can just multiply by it. */
  sign: () => number
}

/**
 * How a list remembers its own sort across visits. `key` names the pair of
 * storage entries (`${key}-field` / `${key}-dir`); `fields` is the list's own
 * sortable columns, guarding a restored value against one a removed enum
 * member left behind.
 */
export interface ListSortPersistence<F extends string> {
  key: string
  fields: readonly F[]
}

/**
 * The sort state behind the agents, repositories and schedules list views.
 * All three carried an identical `sortField`/`sortDir` pair and an identical
 * `toggleSort`; only the set of sortable columns differed.
 *
 * `persist`, when given, restores the field and direction a person left this
 * list on and keeps them updated - a sort choice is a setting, not a filter
 * that should reset itself on every visit.
 */
export function useListSort<F extends string>(
  initial: F,
  initialDir: SortDir = 'asc',
  persist?: ListSortPersistence<F>,
): ListSort<F> {
  const field = persist
    ? usePersistedRef(`${persist.key}-field`, initial, isKnownField(persist.fields))
    : (ref(initial) as Ref<F>)
  const direction = persist
    ? usePersistedRef(`${persist.key}-dir`, initialDir, isSortDir)
    : ref<SortDir>(initialDir)

  function toggle(next: F): void {
    if (field.value === next) {
      direction.value = direction.value === 'asc' ? 'desc' : 'asc'
    } else {
      field.value = next
      direction.value = 'asc'
    }
  }

  function sign(): number {
    return direction.value === 'desc' ? -1 : 1
  }

  return { field, direction, toggle, sign }
}

function isKnownField<F extends string>(fields: readonly F[]): (value: string) => value is F {
  const known: readonly string[] = fields
  return (value: string): value is F => known.includes(value)
}
