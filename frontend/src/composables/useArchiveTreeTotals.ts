// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, type Ref } from 'vue'
import { getArchiveTreeTotals } from '../api/archives'
import { normalizeIndexStatus } from '../utils/archiveIndexStatus'
import type { PathTotals } from '../types/generated'

/** Most paths the server adds up in one request. */
export const TOTALS_BATCH = 1000

interface UseArchiveTreeTotalsReturn {
  /** What each path asked about so far holds, by archive path. */
  totals: Ref<ReadonlyMap<string, PathTotals>>
  /** Asks the content index for every path in `paths` not already known. */
  ensure: (paths: readonly string[]) => Promise<void>
}

/**
 * How many files and bytes paths of one archive hold, read from its content
 * index and kept until another archive opens. A folder's size is what is
 * under it, which a directory listing does not say.
 *
 * Totals only add to what the browser shows, so a failed or not-yet-indexed
 * read leaves a path unknown rather than reporting an error.
 */
export function useArchiveTreeTotals(
  repoId: () => number | null,
  archiveName: () => string | null,
): UseArchiveTreeTotalsReturn {
  const totals = ref<ReadonlyMap<string, PathTotals>>(new Map())
  const asked = new Set<string>()
  // Bumped whenever another archive opens, so an answer about the one left
  // behind is dropped instead of landing in the new archive's totals.
  let generation = 0

  watch(
    () => `${repoId()}/${archiveName()}`,
    () => {
      generation++
      totals.value = new Map()
      asked.clear()
    },
  )

  async function ensure(paths: readonly string[]): Promise<void> {
    const repo = repoId()
    const archive = archiveName()
    if (repo === null || archive === null) return
    const missing = [...new Set(paths)].filter((path) => !asked.has(path))
    if (missing.length === 0) return
    missing.forEach((path) => asked.add(path))
    const started = generation

    const batches = Array.from({ length: Math.ceil(missing.length / TOTALS_BATCH) }, (_, i) =>
      missing.slice(i * TOTALS_BATCH, (i + 1) * TOTALS_BATCH),
    )
    await Promise.all(
      batches.map(async (batch) => {
        try {
          const response = await getArchiveTreeTotals(repo, archive, batch)
          if (started !== generation) return
          if (normalizeIndexStatus(response.index_status) !== 'done') {
            batch.forEach((path) => asked.delete(path))
            return
          }
          const next = new Map(totals.value)
          response.totals.forEach((total) => next.set(total.path, total))
          totals.value = next
        } catch {
          // Left unknown: the next listing that shows these paths asks again.
          if (started === generation) batch.forEach((path) => asked.delete(path))
        }
      }),
    )
  }

  return { totals, ensure }
}
