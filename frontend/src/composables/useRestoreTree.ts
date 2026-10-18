// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, type ComputedRef, type Ref } from 'vue'
import axios from 'axios'
import {
  getArchiveContents,
  getArchiveIndexStatus,
  getArchiveTreeTotals,
  searchArchiveTree,
  type ArchiveTreeSearchOptions,
} from '../api/archives'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { parentOf, type ChildrenOf } from '../utils/restoreSelection'
import type { ContentEntryResponse as ContentEntry, PathTotals } from '../types/generated'

/** Entries a directory shows before a "Load more" row. */
export const TREE_PAGE_SIZE = 200
/** Asked for when a directory's complete listing is needed to split a ticked folder. */
const FULL_LISTING_LIMIT = 100_000
/** Most paths one totals request may name; the server's own limit. */
const TOTALS_BATCH = 1000
const INDEX_POLL_MS = 2000

// borg's raw entry-type character ("d" directory, "-" file, "l" symlink...);
// an external format contract, not app-owned domain state.
const DIRECTORY_ENTRY_TYPE = 'd'

export type IndexState = 'pending' | 'indexing' | 'done' | 'failed'

function normalizeIndexState(status: string): IndexState {
  if (status === 'done') return 'done'
  if (status === 'failed') return 'failed'
  if (status === 'indexing') return 'indexing'
  return 'pending'
}

/** How a file differs from the same path in the previous archive of its host. */
export type ChangeKind = 'new' | 'changed'

export function isDirEntry(entry: Pick<ContentEntry, 'type'>): boolean {
  return entry.type === DIRECTORY_ENTRY_TYPE
}

export function entryName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

/** One entry of the tree as it is drawn: indented under its parent. */
export interface EntryRow {
  kind: 'entry'
  key: string
  path: string
  name: string
  depth: number
  isDir: boolean
  entry: ContentEntry
  /** Whether the row has a chevron: a folder whose children the tree can show. */
  expandable: boolean
  expanded: boolean
  loading: boolean
  /** Whether the entry itself matched the active search, not just a descendant. */
  match: boolean
}

/** The row under a directory's loaded entries when it holds more. */
export interface MoreRow {
  kind: 'more'
  key: string
  dir: string
  depth: number
  loading: boolean
}

export type TreeRow = EntryRow | MoreRow

interface DirListing {
  entries: ContentEntry[]
  complete: boolean
  limit: number
}

interface ArchiveRef {
  repoId: number
  name: string
}

function compareEntries(a: ContentEntry, b: ContentEntry): number {
  const dirOrder = Number(isDirEntry(b)) - Number(isDirEntry(a))
  return dirOrder !== 0 ? dirOrder : entryName(a.path).localeCompare(entryName(b.path))
}

/** Where the tree remembers that a folder opened by a search was closed. */
function closedKey(dir: string): string {
  return `\u0000closed:${dir}`
}

function syntheticDir(path: string): ContentEntry {
  return { type: DIRECTORY_ENTRY_TYPE, path, size: 0, mtime: '', mode: '' }
}

interface UseRestoreTreeReturn {
  indexState: Ref<IndexState>
  error: Ref<string | null>
  rootLoading: ComputedRef<boolean>
  rows: ComputedRef<TreeRow[]>
  searchActive: ComputedRef<boolean>
  searchLoading: Ref<boolean>
  searchError: Ref<string | null>
  searchTruncated: Ref<boolean>
  searchCount: ComputedRef<number>
  totals: Ref<Record<string, PathTotals>>
  rootChildren: ComputedRef<string[]>
  childrenOf: ChildrenOf
  entryOf: (path: string) => ContentEntry | null
  changeOf: (entry: ContentEntry) => ChangeKind | null
  open: (archive: ArchiveRef | null, previous: ArchiveRef | null) => Promise<void>
  toggleExpand: (dir: string) => Promise<void>
  loadMore: (dir: string) => Promise<void>
  ensureListings: (dirs: readonly string[]) => Promise<void>
  ensureTotals: (paths: readonly string[]) => Promise<void>
  search: (options: ArchiveTreeSearchOptions | null) => Promise<void>
  dispose: () => void
}

/**
 * The restore view's tree over one archive's content index: directories load
 * as they are opened, a page at a time; each loaded folder gets its file and
 * byte totals; files are compared with the previous archive of the same host;
 * and a search replaces the browse tree with its matches and their parents.
 */
export function useRestoreTree(): UseRestoreTreeReturn {
  const current = ref<ArchiveRef | null>(null)
  const previous = ref<ArchiveRef | null>(null)
  const indexState = ref<IndexState>('pending')
  const error = ref<string | null>(null)

  const listings = ref<Record<string, DirListing>>({})
  const expanded = ref<Record<string, boolean>>({})
  const loadingDirs = ref<Record<string, boolean>>({})
  const known = ref<Record<string, ContentEntry>>({})
  const totals = ref<Record<string, PathTotals>>({})
  const previousListings = ref<Record<string, Record<string, ContentEntry>>>({})

  const searchResults = ref<ContentEntry[] | null>(null)
  const searchLoading = ref(false)
  const searchError = ref<string | null>(null)
  const searchTruncated = ref(false)

  // Each open() bumps the generation, so a response for an archive that has
  // since been replaced is dropped instead of landing in the new tree.
  let generation = 0
  let pollTimer: ReturnType<typeof setInterval> | null = null
  let searchAbort: AbortController | null = null
  const pendingTotals = new Set<string>()

  function stopPolling(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  function remember(entries: readonly ContentEntry[]): void {
    const next = { ...known.value }
    for (const entry of entries) next[entry.path] = entry
    known.value = next
  }

  function childrenOf(dir: string): readonly string[] | null {
    const listing = listings.value[dir]
    return listing?.complete === true ? listing.entries.map((entry) => entry.path) : null
  }

  function entryOf(path: string): ContentEntry | null {
    return known.value[path] ?? null
  }

  function changeOf(entry: ContentEntry): ChangeKind | null {
    if (isDirEntry(entry)) return null
    const before = previousListings.value[parentOf(entry.path)]
    if (before === undefined) return null
    const old = before[entryName(entry.path)]
    if (old === undefined) return 'new'
    return old.size !== entry.size || old.mtime !== entry.mtime ? 'changed' : null
  }

  async function ensureTotals(paths: readonly string[]): Promise<void> {
    const archive = current.value
    if (archive === null) return
    const wanted = [...new Set(paths)].filter(
      (path) => path !== '' && totals.value[path] === undefined && !pendingTotals.has(path),
    )
    if (wanted.length === 0) return
    wanted.forEach((path) => pendingTotals.add(path))
    const gen = generation
    try {
      for (let i = 0; i < wanted.length; i += TOTALS_BATCH) {
        const batch = wanted.slice(i, i + TOTALS_BATCH)
        const res = await getArchiveTreeTotals(archive.repoId, archive.name, batch)
        if (gen !== generation) return
        const next = { ...totals.value }
        for (const total of res.totals) next[total.path] = total
        totals.value = next
      }
    } catch (e: unknown) {
      logger.warn('could not load folder totals', e)
    } finally {
      wanted.forEach((path) => pendingTotals.delete(path))
    }
  }

  async function loadPrevious(dir: string): Promise<void> {
    const before = previous.value
    if (before === null || previousListings.value[dir] !== undefined) return
    const gen = generation
    try {
      const res = await getArchiveContents(before.repoId, before.name, {
        path: dir === '' ? undefined : dir,
        limit: FULL_LISTING_LIMIT,
      })
      // Only a finished index says what the previous archive held; anything
      // else would mark every file new.
      if (gen !== generation || normalizeIndexState(res.index_status) !== 'done') return
      const byName: Record<string, ContentEntry> = {}
      for (const entry of res.entries) byName[entryName(entry.path)] = entry
      previousListings.value = { ...previousListings.value, [dir]: byName }
    } catch (e: unknown) {
      logger.warn('could not compare with the previous archive', e)
    }
  }

  function startPolling(): void {
    stopPolling()
    const archive = current.value
    if (archive === null) return
    const gen = generation
    pollTimer = setInterval(async () => {
      try {
        const res = await getArchiveIndexStatus(archive.repoId, archive.name)
        if (gen !== generation) return
        const state = normalizeIndexState(res.status)
        indexState.value = state
        if (state === 'done') {
          stopPolling()
          await loadDir('', TREE_PAGE_SIZE)
        } else if (state === 'failed') {
          stopPolling()
          error.value = res.error ?? 'Archive indexing failed'
        }
      } catch (e: unknown) {
        stopPolling()
        error.value = extractError(e)
      }
    }, INDEX_POLL_MS)
  }

  async function loadDir(dir: string, limit: number): Promise<void> {
    const archive = current.value
    if (archive === null) return
    const gen = generation
    loadingDirs.value = { ...loadingDirs.value, [dir]: true }
    try {
      const res = await getArchiveContents(archive.repoId, archive.name, {
        path: dir === '' ? undefined : dir,
        limit,
      })
      if (gen !== generation) return
      const state = normalizeIndexState(res.index_status)
      indexState.value = state
      if (state === 'pending' || state === 'indexing') {
        startPolling()
        return
      }
      const entries = res.entries
        .filter((entry) => entry.path !== dir && parentOf(entry.path) === dir)
        .sort(compareEntries)
      listings.value = {
        ...listings.value,
        [dir]: { entries, complete: res.entries.length < limit, limit },
      }
      remember(entries)
      const childDirs = entries.filter(isDirEntry).map((entry) => entry.path)
      await Promise.all([ensureTotals(childDirs), loadPrevious(dir)])
    } catch (e: unknown) {
      if (gen === generation) error.value = extractError(e)
    } finally {
      if (gen === generation) {
        const next = { ...loadingDirs.value }
        delete next[dir]
        loadingDirs.value = next
      }
    }
  }

  async function open(archive: ArchiveRef | null, before: ArchiveRef | null): Promise<void> {
    generation++
    stopPolling()
    searchAbort?.abort()
    current.value = archive
    previous.value = before
    indexState.value = 'pending'
    error.value = null
    listings.value = {}
    expanded.value = {}
    loadingDirs.value = {}
    known.value = {}
    totals.value = {}
    previousListings.value = {}
    searchResults.value = null
    searchError.value = null
    searchTruncated.value = false
    pendingTotals.clear()
    if (archive !== null) await loadDir('', TREE_PAGE_SIZE)
  }

  async function toggleExpand(dir: string): Promise<void> {
    if (expanded.value[dir] === true) {
      const next = { ...expanded.value }
      delete next[dir]
      expanded.value = next
      return
    }
    expanded.value = { ...expanded.value, [dir]: true }
    if (listings.value[dir] === undefined && searchResults.value === null) {
      await loadDir(dir, TREE_PAGE_SIZE)
    }
  }

  async function loadMore(dir: string): Promise<void> {
    const listing = listings.value[dir]
    await loadDir(dir, (listing?.limit ?? 0) + TREE_PAGE_SIZE)
  }

  async function ensureListings(dirs: readonly string[]): Promise<void> {
    await Promise.all(
      dirs
        .filter((dir) => listings.value[dir]?.complete !== true)
        .map((dir) => loadDir(dir, FULL_LISTING_LIMIT)),
    )
  }

  async function search(options: ArchiveTreeSearchOptions | null): Promise<void> {
    searchAbort?.abort()
    searchAbort = null
    const archive = current.value
    if (options === null || archive === null) {
      searchResults.value = null
      searchError.value = null
      searchTruncated.value = false
      searchLoading.value = false
      return
    }
    const controller = new AbortController()
    searchAbort = controller
    searchLoading.value = true
    searchError.value = null
    try {
      const res = await searchArchiveTree(archive.repoId, archive.name, options, controller.signal)
      if (controller.signal.aborted) return
      const state = normalizeIndexState(res.index_status)
      if (state !== 'done') {
        searchError.value =
          state === 'failed'
            ? 'This archive has no content index, so it cannot be searched.'
            : 'The archive is still being indexed. Search again in a moment.'
        searchResults.value = []
        return
      }
      searchResults.value = res.entries
      searchTruncated.value = res.truncated
      remember(res.entries)
      await ensureTotals(res.entries.filter(isDirEntry).map((entry) => entry.path))
    } catch (e: unknown) {
      if (axios.isCancel(e) || controller.signal.aborted) return
      searchError.value = extractError(e)
      searchResults.value = []
    } finally {
      if (searchAbort === controller) {
        searchLoading.value = false
        searchAbort = null
      }
    }
  }

  function browseRows(): TreeRow[] {
    const rows: TreeRow[] = []
    const walk = (dir: string, depth: number): void => {
      const listing = listings.value[dir]
      if (listing === undefined) return
      for (const entry of listing.entries) {
        const isDir = isDirEntry(entry)
        const open = isDir && expanded.value[entry.path] === true
        rows.push({
          kind: 'entry',
          key: entry.path,
          path: entry.path,
          name: entryName(entry.path),
          depth,
          isDir,
          entry,
          expandable: isDir,
          expanded: open,
          loading: loadingDirs.value[entry.path] === true,
          match: false,
        })
        if (open) walk(entry.path, depth + 1)
      }
      if (!listing.complete) {
        rows.push({
          kind: 'more',
          key: `${dir}\u0000more`,
          dir,
          depth,
          loading: loadingDirs.value[dir] === true,
        })
      }
    }
    walk('', 0)
    return rows
  }

  function searchRows(results: readonly ContentEntry[]): TreeRow[] {
    const matched = new Set(results.map((entry) => entry.path))
    const nodes = new Map<string, ContentEntry>()
    for (const entry of results) {
      nodes.set(entry.path, entry)
      let dir = parentOf(entry.path)
      while (dir !== '' && !nodes.has(dir)) {
        nodes.set(dir, known.value[dir] ?? syntheticDir(dir))
        dir = parentOf(dir)
      }
    }
    const children = new Map<string, ContentEntry[]>()
    for (const entry of nodes.values()) {
      const parent = parentOf(entry.path)
      children.set(parent, [...(children.get(parent) ?? []), entry])
    }
    const rows: TreeRow[] = []
    const walk = (dir: string, depth: number): void => {
      for (const entry of [...(children.get(dir) ?? [])].sort(compareEntries)) {
        // The folders on the way to a match start open. A search shows only
        // matches, so a matched folder with nothing matching inside it has
        // nothing to open; ticking it still takes everything it holds.
        const hasChildren = children.has(entry.path)
        const open = hasChildren && expanded.value[closedKey(entry.path)] !== true
        rows.push({
          kind: 'entry',
          key: entry.path,
          path: entry.path,
          name: entryName(entry.path),
          depth,
          isDir: isDirEntry(entry),
          entry,
          expandable: hasChildren,
          expanded: open,
          loading: false,
          match: matched.has(entry.path),
        })
        if (open) walk(entry.path, depth + 1)
      }
    }
    walk('', 0)
    return rows
  }

  const rows = computed<TreeRow[]>(() =>
    searchResults.value === null ? browseRows() : searchRows(searchResults.value),
  )

  const searchActive = computed(() => searchResults.value !== null)
  const searchCount = computed(() => searchResults.value?.length ?? 0)
  const rootLoading = computed(() => loadingDirs.value[''] === true)
  const rootChildren = computed(() =>
    (listings.value['']?.entries ?? []).map((entry) => entry.path),
  )

  /**
   * In search mode a folder on the way to a match starts open, so closing it
   * is the thing remembered; everywhere else opening is.
   */
  async function toggleExpandAware(dir: string): Promise<void> {
    if (searchResults.value !== null) {
      if (searchResults.value.some((entry) => entry.path.startsWith(`${dir}/`))) {
        const key = closedKey(dir)
        const next = { ...expanded.value }
        if (next[key] === true) delete next[key]
        else next[key] = true
        expanded.value = next
        return
      }
    }
    await toggleExpand(dir)
  }

  function dispose(): void {
    generation++
    stopPolling()
    searchAbort?.abort()
  }

  return {
    indexState,
    error,
    rootLoading,
    rows,
    searchActive,
    searchLoading,
    searchError,
    searchTruncated,
    searchCount,
    totals,
    rootChildren,
    childrenOf,
    entryOf,
    changeOf,
    open,
    toggleExpand: toggleExpandAware,
    loadMore,
    ensureListings,
    ensureTotals,
    search,
    dispose,
  }
}
