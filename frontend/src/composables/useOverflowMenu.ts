// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { onBeforeUnmount, ref, watch, type Ref } from 'vue'
import { useEscapeKey } from './useEscapeKey'

export interface OverflowMenu {
  /** Whether the menu is open. Bind a toggle button's click to flip it. */
  menuOpen: Ref<boolean>
  /** Runs a menu action and closes the menu, so no item has to remember to. */
  runAndClose: (action: () => void) => void
}

/**
 * The open/close behavior shared by every header's overflow menu (AgentHeader,
 * ScheduleHeader): Escape and an outside click both close it, and choosing an
 * item closes it too. Extracted once both headers grew the identical 25-line
 * block independently.
 *
 * Takes the root element ref rather than creating it: Vue's `ref="x"` template
 * attribute only wires up a template ref when `x` is a plain top-level
 * `ref()` binding in the component's own `<script setup>`, so the caller
 * declares it and hands it in.
 */
export function useOverflowMenu(root: Ref<HTMLElement | null>): OverflowMenu {
  const open = ref(false)

  useEscapeKey(open, () => {
    open.value = false
  })

  function onDocumentPointerDown(e: PointerEvent): void {
    if (!root.value?.contains(e.target as Node)) open.value = false
  }

  watch(open, (isOpen) => {
    if (isOpen) document.addEventListener('pointerdown', onDocumentPointerDown)
    else document.removeEventListener('pointerdown', onDocumentPointerDown)
  })

  onBeforeUnmount(() => {
    document.removeEventListener('pointerdown', onDocumentPointerDown)
  })

  function runAndClose(action: () => void): void {
    open.value = false
    action()
  }

  return { menuOpen: open, runAndClose }
}
