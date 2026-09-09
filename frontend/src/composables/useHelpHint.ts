// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, type Ref } from 'vue'
import { useOverflowMenu } from './useOverflowMenu'

/** The one `HelpHint` open at a time, across every instance on the page. */
const activeHint = ref<symbol | null>(null)

/**
 * Open/close behavior for one `HelpHint` popover: everything `useOverflowMenu`
 * already gives a menu - Escape, a click outside the button closes it - plus
 * closing a hint when a different one opens. An outside click covers that for
 * a pointer (clicking button B is outside button A's root), but not for a
 * keyboard activation of button B, so it is handled explicitly here rather
 * than left to rely on click order.
 */
export function useHelpHint(root: Ref<HTMLElement | null>): Ref<boolean> {
  const id = Symbol('help-hint')
  const { menuOpen: open } = useOverflowMenu(root)

  watch(open, (isOpen) => {
    if (isOpen) {
      activeHint.value = id
    } else if (activeHint.value === id) {
      activeHint.value = null
    }
  })

  watch(activeHint, (current) => {
    if (current !== id) open.value = false
  })

  return open
}
