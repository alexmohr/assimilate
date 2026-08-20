// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, type ComputedRef, type Ref } from 'vue'
import type { ArchiveEntry } from './useArchiveBrowser'

export type ArchiveSortMode =
  | 'date-desc'
  | 'date-asc'
  | 'size-desc'
  | 'size-asc'
  | 'dedup-desc'
  | 'dedup-asc'

/**
 * Above this many hosts the grouped list stops being something you read and
 * becomes something you navigate, so the groups start closed. At or below it -
 * a schedule targeting two or three hosts, a repository with one - starting
 * closed just hides the whole list behind a click.
 */
export const GROUP_COLLAPSE_THRESHOLD = 3

export interface ArchiveGroup {
  hostname: string
  matched: boolean
  agentHostname: string | null
  archives: ArchiveEntry[]
}

export const ARCHIVE_SORT_OPTIONS: { value: ArchiveSortMode; label: string }[] = [
  { value: 'date-desc', label: 'Date newest first' },
  { value: 'date-asc', label: 'Date oldest first' },
  { value: 'size-desc', label: 'Size largest first' },
  { value: 'size-asc', label: 'Size smallest first' },
  { value: 'dedup-desc', label: 'Dedup largest first' },
  { value: 'dedup-asc', label: 'Dedup smallest first' },
]

export interface UseArchiveList {
  filter: Ref<string>
  sortMode: Ref<ArchiveSortMode>
  groupByHost: Ref<boolean>
  ordered: ComputedRef<ArchiveEntry[]>
  grouped: ComputedRef<ArchiveGroup[]>
  unmatchedCount: ComputedRef<number>
  unmatchedHostnames: ComputedRef<string[]>
  toggleGroup: (hostname: string) => void
  isGroupCollapsed: (hostname: string) => boolean
  reset: () => void
}

/**
 * Filtering, ordering and host-grouping for a repository's archive list.
 *
 * Pure derivation over the archives it is given: no fetching, no deletion, no
 * router. Split out of RepoDetailView, which held all of it inline.
 */
export function useArchiveList(
  archives: Ref<ArchiveEntry[]> | ComputedRef<ArchiveEntry[]>,
): UseArchiveList {
  const filter = ref('')
  const sortMode = ref<ArchiveSortMode>('date-desc')
  const groupByHost = ref(true)
  // Groups the user has clicked, held as "the opposite of the default" rather
  // than as "expanded": the default flips with the number of hosts, and a
  // user's own click has to survive that flip.
  const toggledGroups = ref<Set<string>>(new Set())

  const unmatchedCount = computed(() => archives.value.filter((a) => a.matched !== true).length)

  const unmatchedHostnames = computed(() => [
    ...new Set(archives.value.filter((a) => a.matched !== true).map((a) => a.hostname)),
  ])

  const filtered = computed<ArchiveEntry[]>(() => {
    const needle = filter.value.toLowerCase()
    return needle
      ? archives.value.filter(
          (a) => a.name.toLowerCase().includes(needle) || a.hostname.toLowerCase().includes(needle),
        )
      : archives.value
  })

  const ordered = computed<ArchiveEntry[]>(() => {
    const byDate = (l: ArchiveEntry, r: ArchiveEntry): number => l.start.localeCompare(r.start)
    const bySize = (l: ArchiveEntry, r: ArchiveEntry): number => l.original_size - r.original_size
    const byDedup = (l: ArchiveEntry, r: ArchiveEntry): number =>
      l.deduplicated_size - r.deduplicated_size

    const list = [...filtered.value]
    switch (sortMode.value) {
      case 'date-desc':
        return list.sort((a, b) => byDate(b, a))
      case 'date-asc':
        return list.sort(byDate)
      case 'size-desc':
        return list.sort((a, b) => bySize(b, a))
      case 'size-asc':
        return list.sort(bySize)
      case 'dedup-desc':
        return list.sort((a, b) => byDedup(b, a))
      case 'dedup-asc':
        return list.sort(byDedup)
      default:
        return list
    }
  })

  const grouped = computed<ArchiveGroup[]>(() => {
    const groups = new Map<string, ArchiveGroup>()
    for (const archive of ordered.value) {
      // An unmatched archive is grouped under the hostname borg recorded,
      // since there is no agent to attribute it to yet.
      const isMatched = archive.matched === true
      const key = isMatched ? (archive.agent_hostname ?? archive.hostname) : archive.hostname
      if (!groups.has(key)) {
        groups.set(key, {
          hostname: key,
          matched: isMatched,
          agentHostname: isMatched ? archive.agent_hostname : null,
          archives: [],
        })
      }
      groups.get(key)!.archives.push(archive)
    }
    return [...groups.values()].sort((a, b) => a.hostname.localeCompare(b.hostname))
  })

  const collapsedByDefault = computed(() => grouped.value.length > GROUP_COLLAPSE_THRESHOLD)

  function toggleGroup(hostname: string): void {
    const next = new Set(toggledGroups.value)
    if (next.has(hostname)) next.delete(hostname)
    else next.add(hostname)
    toggledGroups.value = next
  }

  function isGroupCollapsed(hostname: string): boolean {
    const flipped = toggledGroups.value.has(hostname)
    return collapsedByDefault.value ? !flipped : flipped
  }

  /** Clears the filter and collapse state, e.g. when the repository changes. */
  function reset(): void {
    filter.value = ''
    toggledGroups.value = new Set()
  }

  return {
    filter,
    sortMode,
    groupByHost,
    ordered,
    grouped,
    unmatchedCount,
    unmatchedHostnames,
    toggleGroup,
    isGroupCollapsed,
    reset,
  }
}
