// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, onBeforeUnmount, onMounted, ref, type ComputedRef } from 'vue'

/**
 * Tracks the `<html>` class attribute (toggled by dark-mode) so chart.js
 * color options can react to a theme switch without a page reload.
 */
export function useChartTheme(): {
  textMuted: ComputedRef<string>
  border: ComputedRef<string>
} {
  const generation = ref(0)
  let observer: MutationObserver | null = null

  onMounted(() => {
    observer = new MutationObserver(() => {
      generation.value++
    })
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    })
  })

  onBeforeUnmount(() => {
    observer?.disconnect()
  })

  function cssVar(name: string): string {
    void generation.value
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  }

  return {
    textMuted: computed(() => cssVar('--text-muted')),
    border: computed(() => cssVar('--border')),
  }
}
