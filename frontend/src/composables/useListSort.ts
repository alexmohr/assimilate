// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'

export type SortDir = 'asc' | 'desc'

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
 * The sort state behind the agents, repositories and schedules list views.
 * All three carried an identical `sortField`/`sortDir` pair and an identical
 * `toggleSort`; only the set of sortable columns differed.
 */
export function useListSort<F extends string>(
  initial: F,
  initialDir: SortDir = 'asc',
): ListSort<F> {
  const field = ref(initial) as Ref<F>
  const direction = ref<SortDir>(initialDir)

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
