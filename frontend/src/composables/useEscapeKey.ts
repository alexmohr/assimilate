// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { onUnmounted, watch, type Ref } from 'vue'

export function useEscapeKey(active: Ref<boolean>, callback: () => void): void {
  function handler(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      callback()
    }
  }

  watch(
    active,
    (val) => {
      if (val) {
        window.addEventListener('keydown', handler)
      } else {
        window.removeEventListener('keydown', handler)
      }
    },
    { immediate: true },
  )

  onUnmounted(() => {
    window.removeEventListener('keydown', handler)
  })
}
