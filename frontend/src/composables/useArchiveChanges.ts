// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, watch, type ComputedRef } from 'vue'
import { getArchiveContents } from '../api/archives'
import { normalizeIndexStatus } from '../utils/archiveIndexStatus'
import { changeOf as compare, previousArchive, type ChangeKind } from '../utils/archiveChanges'
import { directChildren, parentOf } from '../utils/archiveSelection'
import type { ArchiveEntryResponse, ContentEntryResponse as ContentEntry } from '../types/generated'

/** Entries asked for from the previous archive's folder: a shorter answer is all of it. */
export const PREVIOUS_LISTING_LIMIT = 10_000

interface UseArchiveChangesReturn {
  /** The archive the open one is compared with, or null when there is none. */
  previous: ComputedRef<ArchiveEntryResponse | null>
  /** Reads what folder `dir` held in the previous archive, if not read already. */
  load: (dir: string) => Promise<void>
  /** How `entry` differs from the previous archive, once its folder there is known. */
  changeOf: (entry: ContentEntry) => ChangeKind | null
}

/**
 * Compares the open archive's files with the previous archive of the same
 * machine, one folder at a time as the browser shows it. The previous
 * folder's listing comes from that archive's content index; until the index
 * is done, or when a folder holds more than one request returns, its files
 * are left unmarked rather than guessed at.
 */
export function useArchiveChanges(
  repoId: () => number | null,
  archive: () => ArchiveEntryResponse | null,
  archives: () => readonly ArchiveEntryResponse[],
): UseArchiveChangesReturn {
  const previous = computed(() => {
    const current = archive()
    return current === null ? null : previousArchive(archives(), current)
  })
  const listings = ref<ReadonlyMap<string, ReadonlyMap<string, ContentEntry>>>(new Map())
  const asked = new Set<string>()
  let generation = 0

  watch(
    () => `${repoId()}/${archive()?.name ?? ''}/${previous.value?.name ?? ''}`,
    () => {
      generation++
      listings.value = new Map()
      asked.clear()
    },
  )

  async function load(dir: string): Promise<void> {
    const repo = repoId()
    const before = previous.value
    if (repo === null || before === null || asked.has(dir)) return
    asked.add(dir)
    const started = generation
    try {
      const response = await getArchiveContents(repo, before.name, {
        path: dir,
        limit: PREVIOUS_LISTING_LIMIT,
      })
      if (started !== generation) return
      if (
        normalizeIndexStatus(response.index_status) !== 'done' ||
        response.entries.length >= PREVIOUS_LISTING_LIMIT
      ) {
        asked.delete(dir)
        return
      }
      const children = directChildren(response.entries, dir).map(
        (entry): [string, ContentEntry] => [entry.path, entry],
      )
      listings.value = new Map(listings.value).set(dir, new Map(children))
    } catch {
      // Badges only add to the listing; the next visit to the folder asks again.
      if (started === generation) asked.delete(dir)
    }
  }

  function changeOf(entry: ContentEntry): ChangeKind | null {
    return compare(entry, listings.value.get(parentOf(entry.path)))
  }

  return { previous, load, changeOf }
}
