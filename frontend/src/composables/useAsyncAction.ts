// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { extractBlobError } from '../utils/error'

/** True while the call it was handed to has not been superseded. */
export type IsCurrent = () => boolean

interface UseAsyncActionReturn {
  loading: Ref<boolean>
  error: Ref<string | null>
  run: <T>(fn: () => Promise<T>) => Promise<T | undefined>
  runLatest: <T>(fn: (isCurrent: IsCurrent) => Promise<T>) => Promise<T | undefined>
  latestGuard: () => IsCurrent
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
  let generation = 0

  async function run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    loading.value = true
    error.value = null
    try {
      return await fn()
    } catch (e) {
      error.value = await extractBlobError(e, context)
      return undefined
    } finally {
      loading.value = false
    }
  }

  /**
   * `run` for a load that a later one replaces - a detail page re-fetching
   * because its route param changed. Only the most recent call may clear
   * `loading` or set `error`, so an earlier load that settles late neither
   * hides the spinner of the one still in flight nor puts its failure on the
   * page that replaced it. `fn` receives `isCurrent` to check before it
   * writes anything of its own.
   */
  async function runLatest<T>(fn: (isCurrent: IsCurrent) => Promise<T>): Promise<T | undefined> {
    const mine = ++generation
    const isCurrent: IsCurrent = () => mine === generation
    loading.value = true
    error.value = null
    try {
      return await fn(isCurrent)
    } catch (e) {
      const message = await extractBlobError(e, context)
      if (isCurrent()) error.value = message
      return undefined
    } finally {
      if (isCurrent()) loading.value = false
    }
  }

  /**
   * A guard for work started outside `runLatest` - a background refresh -
   * that goes stale as soon as the next `runLatest` call begins.
   */
  function latestGuard(): IsCurrent {
    const at = generation
    return () => at === generation
  }

  return { loading, error, run, runLatest, latestGuard }
}
