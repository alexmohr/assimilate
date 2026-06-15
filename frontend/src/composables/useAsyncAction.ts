// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { extractError } from '../utils/error'

interface UseAsyncActionReturn {
  loading: Ref<boolean>
  error: Ref<string | null>
  run: <T>(fn: () => Promise<T>) => Promise<T | undefined>
}

/**
 * Wraps an async operation with shared loading and error state.
 *
 * `run` sets `loading` while the operation is in flight, clears any previous
 * error, and on failure stores a human-readable message via `extractError`
 * and resolves to `undefined` instead of throwing.
 */
export function useAsyncAction(context?: string): UseAsyncActionReturn {
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    loading.value = true
    error.value = null
    try {
      return await fn()
    } catch (e) {
      error.value = extractError(e, context)
      return undefined
    } finally {
      loading.value = false
    }
  }

  return { loading, error, run }
}
