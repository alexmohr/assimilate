// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'

interface UseClipboardReturn {
  copied: Ref<boolean>
  copy: (text: string) => Promise<void>
}

export function useClipboard(timeout: number = 2000): UseClipboardReturn {
  const copied = ref(false)

  async function copy(text: string): Promise<void> {
    await navigator.clipboard.writeText(text)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, timeout)
  }

  return { copied, copy }
}
