// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, inject, type ComputedRef } from 'vue'
import { routerKey, type LocationQuery } from 'vue-router'
import { logger } from '../utils/logger'

/** The query key an archive browser keeps its directory under by default. */
export const DEFAULT_ARCHIVE_PATH_QUERY_KEY = 'path'

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

/** `query` with `key` naming `path`, or without `key` at the root. */
function queryWithPath(query: LocationQuery, key: string, path: string): LocationQuery {
  const next: LocationQuery = { ...query }
  if (path === ROOT_PATH) {
    delete next[key]
  } else {
    next[key] = path
  }
  return next
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
