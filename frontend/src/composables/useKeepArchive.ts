// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { keepArchive } from '../api/appendOnly'
import { extractError } from '../utils/error'
import { useToast } from './useToast'

export interface UseKeepArchive {
  /** Whether a request to keep an archive is in flight. */
  keeping: Ref<boolean>
  /**
   * Takes back the deletion queued for an archive, runs `afterKept` so the
   * caller can refresh what it shows, and says so; a failure is toasted.
   */
  keep: (
    repoId: number,
    archiveName: string,
    afterKept: () => Promise<void> | void,
  ) => Promise<void>
}

/** The one place that cancels a queued deletion and reports how it went. */
export function useKeepArchive(): UseKeepArchive {
  const { success: toastSuccess, error: toastError } = useToast()
  const keeping = ref(false)

  async function keep(
    repoId: number,
    archiveName: string,
    afterKept: () => Promise<void> | void,
  ): Promise<void> {
    keeping.value = true
    try {
      await keepArchive(repoId, archiveName)
      await afterKept()
      toastSuccess(`${archiveName} will be kept.`)
    } catch (e: unknown) {
      toastError(extractError(e))
    } finally {
      keeping.value = false
    }
  }

  return { keeping, keep }
}
