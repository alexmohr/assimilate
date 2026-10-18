// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, inject, type ComputedRef } from 'vue'
import { routerKey, type LocationQuery } from 'vue-router'
import { logger } from '../utils/logger'

/** The query key an archive browser keeps its directory under by default. */
export const DEFAULT_ARCHIVE_PATH_QUERY_KEY = 'path'

/**
 * The query key an archive explorer keeps its open archive under by default.
 * Not `archive`: that one already narrows a repository's list to one archive.
 */
export const DEFAULT_ARCHIVE_NAME_QUERY_KEY = 'browse'

const ROOT_PATH = '/'
const CURRENT_DIR_SEGMENT = '.'
const PARENT_DIR_SEGMENT = '..'

/**
 * Reads a directory inside an archive from a route query value, in the
 * browser's own form: `/` for the root, otherwise `/a/b` with no trailing
 * slash.
 *
 * The value comes straight from the address bar, so anything that is not one
 * plain string, or that tries to climb out with `..`, opens the root instead.
 * Empty and `.` segments are dropped, the way a shell reads `a//./b`.
 */
export function parseArchivePath(value: unknown): string {
  if (typeof value !== 'string') return ROOT_PATH
  const segments = value.split('/').filter((s) => s.length > 0 && s !== CURRENT_DIR_SEGMENT)
  if (segments.includes(PARENT_DIR_SEGMENT)) return ROOT_PATH
  return segments.length === 0 ? ROOT_PATH : `/${segments.join('/')}`
}

/** `query` with `key` set to `value`, or without `key` when `value` is null. */
function queryWith(query: LocationQuery, key: string, value: string | null): LocationQuery {
  const next: LocationQuery = { ...query }
  if (value === null) {
    delete next[key]
  } else {
    next[key] = value
  }
  return next
}

/** `query` with `key` naming `path`, or without `key` at the root. */
function queryWithPath(query: LocationQuery, key: string, path: string): LocationQuery {
  return queryWith(query, key, path === ROOT_PATH ? null : path)
}

/**
 * Reads an archive name from a route query value: one non-empty string, or
 * null for anything else the address bar might hold.
 */
export function parseArchiveName(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null
}

export interface ArchivePathRoute {
  /** False when no router is installed; the browser then keeps its path to itself. */
  enabled: boolean
  /** The directory the route names, `/` when it names none. */
  path: ComputedRef<string>
  /** Opens `path` as a new history entry, so Back returns to the current one. */
  push: (path: string) => void
  /** Drops the directory from the current entry, e.g. when another archive opens. */
  clear: () => void
  /**
   * Drops the directory when the browser goes away while its page stays, e.g.
   * on a tab switch, so it does not reopen in whichever archive is picked next.
   * Leaves the route alone when the browser is unmounting because the app
   * navigated elsewhere.
   */
  release: () => void
}

/**
 * Keeps an archive browser's current directory in the route query, so each
 * directory opened is a browser history entry: Back walks back up through
 * the directories visited until the archive root, then leaves the page as
 * usual, and a link with the query opens the archive at that directory.
 *
 * Every other query parameter is carried over untouched.
 */
export function useArchivePathRoute(queryKey: () => string): ArchivePathRoute {
  // Injected with a default rather than through `useRouter`, so a browser
  // mounted without a router (component tests, embedded previews) still works
  // and just keeps its path in memory.
  const router = inject(routerKey, null)
  const mountedOn = router?.currentRoute.value.path ?? null

  const path = computed<string>(() =>
    router === null ? ROOT_PATH : parseArchivePath(router.currentRoute.value.query[queryKey()]),
  )

  function hasKey(): boolean {
    return router !== null && queryKey() in router.currentRoute.value.query
  }

  function push(target: string): void {
    if (router === null) return
    const query = queryWithPath(router.currentRoute.value.query, queryKey(), target)
    router.push({ query }).catch(logger.error)
  }

  function clear(): void {
    if (router === null || !hasKey()) return
    const query = queryWithPath(router.currentRoute.value.query, queryKey(), ROOT_PATH)
    router.replace({ query }).catch(logger.error)
  }

  function release(): void {
    if (router === null || router.currentRoute.value.path !== mountedOn) return
    clear()
  }

  return { enabled: router !== null, path, push, clear, release }
}

export interface ArchiveSelectionRoute {
  /** False when no router is installed; the explorer then keeps its selection to itself. */
  enabled: boolean
  /** The archive the route names, null when it names none. */
  name: ComputedRef<string | null>
  /**
   * Opens `name` at its root as a new history entry, so Back returns to the
   * archive and directory open now.
   */
  push: (name: string) => void
  /**
   * Points the current entry at `name` (or at no archive), at its root, e.g.
   * when the page itself changes the selection.
   */
  replace: (name: string | null) => void
  /** Drops the archive and its directory when the explorer goes away while its page stays. */
  release: () => void
}

/**
 * Keeps an archive explorer's open archive in the route query next to the
 * directory its browser keeps there, so picking an archive is a browser
 * history entry and a link names both the archive and the directory in it.
 *
 * Every change of archive also drops the directory, in the same navigation:
 * a directory belongs to the archive it was opened in.
 */
export function useArchiveSelectionRoute(
  nameKey: () => string,
  pathKey: () => string,
): ArchiveSelectionRoute {
  const router = inject(routerKey, null)
  const mountedOn = router?.currentRoute.value.path ?? null

  const name = computed<string | null>(() =>
    router === null ? null : parseArchiveName(router.currentRoute.value.query[nameKey()]),
  )

  function queryFor(target: string | null): LocationQuery {
    const query = queryWith(router?.currentRoute.value.query ?? {}, nameKey(), target)
    return queryWith(query, pathKey(), null)
  }

  function push(target: string): void {
    if (router === null) return
    router.push({ query: queryFor(target) }).catch(logger.error)
  }

  // Not skipped when the route already names `target`: a replace also cancels
  // a push still in flight, which is how a selection made and then undone
  // before the push lands keeps the route from reopening it.
  function replace(target: string | null): void {
    if (router === null) return
    router.replace({ query: queryFor(target) }).catch(logger.error)
  }

  function release(): void {
    if (router === null || router.currentRoute.value.path !== mountedOn) return
    const query = router.currentRoute.value.query
    if (!(nameKey() in query) && !(pathKey() in query)) return
    router.replace({ query: queryFor(null) }).catch(logger.error)
  }

  return { enabled: router !== null, name, push, replace, release }
}
