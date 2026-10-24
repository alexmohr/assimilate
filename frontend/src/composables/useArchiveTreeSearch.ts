// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch, type Ref } from 'vue'
import axios from 'axios'
import { searchArchiveTree, type ArchiveTreeSearchOptions } from '../api/archives'
import { normalizeIndexStatus } from '../utils/archiveIndexStatus'
import { extractError } from '../utils/error'
import type { ContentEntryResponse as ContentEntry } from '../types/generated'

/** Most matches one search shows. */
export const SEARCH_LIMIT = 200

interface UseArchiveTreeSearchReturn {
  /** The latest search's matches, in archive path order. */
  entries: Ref<ContentEntry[]>
  /** Whether more entries matched than `entries` holds. */
  truncated: Ref<boolean>
  loading: Ref<boolean>
  error: Ref<string | null>
  /** Whether the archive's index is not done yet, so nothing could be searched. */
  notIndexed: Ref<boolean>
  /** Runs `options` against the open archive's index; `null` clears the results. */
  search: (options: ArchiveTreeSearchOptions | null) => Promise<void>
}

/**
 * Searches the open archive's content index, the whole archive at once.
 * A new search cancels the one still running, so results always answer the
 * latest query, and opening another archive clears them.
 */
export function useArchiveTreeSearch(
  repoId: () => number | null,
  archiveName: () => string | null,
): UseArchiveTreeSearchReturn {
  const entries = ref<ContentEntry[]>([])
  const truncated = ref(false)
  const loading = ref(false)
  const error = ref<string | null>(null)
  const notIndexed = ref(false)
  let controller: AbortController | null = null

  function clear(): void {
    controller?.abort()
    controller = null
    entries.value = []
    truncated.value = false
    loading.value = false
    error.value = null
    notIndexed.value = false
  }

  watch(() => `${repoId()}/${archiveName()}`, clear)

  async function search(options: ArchiveTreeSearchOptions | null): Promise<void> {
    const repo = repoId()
    const archive = archiveName()
    clear()
    if (options === null || repo === null || archive === null) return
    const current = new AbortController()
    controller = current
    loading.value = true
    try {
      const response = await searchArchiveTree(
        repo,
        archive,
        { ...options, limit: SEARCH_LIMIT },
        current.signal,
      )
      if (controller !== current) return
      notIndexed.value = normalizeIndexStatus(response.index_status) !== 'done'
      entries.value = response.entries
      truncated.value = response.truncated
    } catch (e: unknown) {
      if (controller === current && !axios.isCancel(e)) error.value = extractError(e)
    } finally {
      if (controller === current) loading.value = false
    }
  }

  return { entries, truncated, loading, error, notIndexed, search }
}
