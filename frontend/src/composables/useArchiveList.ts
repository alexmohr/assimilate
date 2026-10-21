// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { resolveArchiveHost } from '../utils/archiveHost'
import { archivePeriod } from '../utils/archivePeriod'
import { reposOf, type ArchiveRepo, type RepoArchive } from '../utils/repoArchive'

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

/**
 * How the list is laid out: under one heading per host, under one per stretch
 * of time (`archivePeriod`), or as one flat list.
 */
export type ArchiveGroupMode = 'host' | 'day' | 'flat'

export interface ArchiveGroup {
  hostname: string
  matched: boolean
  agentHostname: string | null
  archives: RepoArchive[]
}

export interface ArchiveDayGroup {
  key: string
  label: string
  archives: RepoArchive[]
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
  /** The repository the list is narrowed to, or null for all of them. */
  repoFilter: Ref<number | null>
  /** The repositories the list spans, each with its archive count. */
  repos: ComputedRef<(ArchiveRepo & { count: number })[]>
  groupMode: Ref<ArchiveGroupMode>
  /** Whether the archives come from more than one host. */
  multiHost: ComputedRef<boolean>
  ordered: ComputedRef<RepoArchive[]>
  grouped: ComputedRef<ArchiveGroup[]>
  dayGroups: ComputedRef<ArchiveDayGroup[]>
  unmatchedCount: ComputedRef<number>
  unmatchedHostnames: ComputedRef<string[]>
  toggleGroup: (hostname: string) => void
  isGroupCollapsed: (hostname: string) => boolean
  reset: () => void
}

/**
 * Filtering, ordering and grouping for an archive list, which may span
 * several repositories.
 *
 * Pure derivation over the archives it is given: no fetching, no deletion, no
 * router. Split out of RepoDetailView, which held all of it inline.
 */
export function useArchiveList(
  archives: Ref<RepoArchive[]> | ComputedRef<RepoArchive[]>,
): UseArchiveList {
  const filter = ref('')
  const sortMode = ref<ArchiveSortMode>('date-desc')
  const repoFilter = ref<number | null>(null)
  // What the user picked, or null for the default the list itself suggests.
  const pickedGroupMode = ref<ArchiveGroupMode | null>(null)
  // Groups the user has clicked, held as "the opposite of the default" rather
  // than as "expanded": the default flips with the number of hosts, and a
  // user's own click has to survive that flip.
  const toggledGroups = ref<Set<string>>(new Set())

  const unmatchedCount = computed(() => archives.value.filter((a) => a.matched !== true).length)

  const unmatchedHostnames = computed(() => [
    ...new Set(archives.value.filter((a) => a.matched !== true).map((a) => a.hostname)),
  ])

  const repos = computed(() => reposOf(archives.value))

  /**
   * The narrowing that is in force: a repository picked from a list that no
   * longer holds it (the caller's repositories changed) narrows to nothing,
   * which would read as "no archives" - so it falls back to all of them.
   */
  const activeRepo = computed<number | null>(() =>
    repos.value.some((repo) => repo.id === repoFilter.value) ? repoFilter.value : null,
  )

  const multiHost = computed(
    () => new Set(archives.value.map((a) => resolveArchiveHost(a))).size > 1,
  )

  /**
   * One host's archives - an agent's Backups tab, most schedules - read best
   * by when they were made; a repository shared by several hosts reads best
   * host by host. One host's archives are never grouped by host, even when
   * the reader picked that while the list still held several: a single group
   * holding everything says nothing.
   */
  const groupMode = computed<ArchiveGroupMode>({
    get: () => {
      const picked = pickedGroupMode.value
      if (picked === null || (picked === 'host' && !multiHost.value)) {
        return multiHost.value ? 'host' : 'day'
      }
      return picked
    },
    set: (mode) => {
      pickedGroupMode.value = mode
    },
  })

  const filtered = computed<RepoArchive[]>(() => {
    const needle = filter.value.toLowerCase()
    const repo = activeRepo.value
    return archives.value.filter(
      (a) =>
        (repo === null || a.repo_id === repo) &&
        (!needle ||
          a.name.toLowerCase().includes(needle) ||
          a.hostname.toLowerCase().includes(needle) ||
          a.repo_name.toLowerCase().includes(needle)),
    )
  })

  const ordered = computed<RepoArchive[]>(() => {
    const byDate = (l: RepoArchive, r: RepoArchive): number => l.start.localeCompare(r.start)
    const bySize = (l: RepoArchive, r: RepoArchive): number => l.original_size - r.original_size
    const byDedup = (l: RepoArchive, r: RepoArchive): number =>
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
      const key = resolveArchiveHost(archive)
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

  // In list order, so a newest-first list reads Today, Yesterday, ... and an
  // oldest-first one the other way round.
  const dayGroups = computed<ArchiveDayGroup[]>(() => {
    const now = new Date()
    const groups = new Map<string, ArchiveDayGroup>()
    for (const archive of ordered.value) {
      const period = archivePeriod(archive.start, now)
      const group = groups.get(period.key) ?? { ...period, archives: [] }
      group.archives.push(archive)
      groups.set(period.key, group)
    }
    return [...groups.values()]
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

  /** Clears the filter, repository and collapse state, e.g. when the repository changes. */
  function reset(): void {
    filter.value = ''
    repoFilter.value = null
    toggledGroups.value = new Set()
  }

  return {
    filter,
    sortMode,
    repoFilter,
    repos,
    groupMode,
    multiHost,
    ordered,
    grouped,
    dayGroups,
    unmatchedCount,
    unmatchedHostnames,
    toggleGroup,
    isGroupCollapsed,
    reset,
  }
}
