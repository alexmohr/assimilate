// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type ComputedRef, type Ref } from 'vue'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useToast } from './useToast'
import type { ArchiveEntry } from './useArchiveBrowser'

export interface UseArchiveDeletionOptions {
  /** The archive list as it currently stands, used to prune stale markers. */
  sortedArchives: Ref<ArchiveEntry[]> | ComputedRef<ArchiveEntry[]>
  deleteArchiveByName: (archive: ArchiveEntry) => Promise<unknown>
  /** Silent reload, so the panel does not flash a loading placeholder. */
  reloadArchives: (silent: boolean) => Promise<unknown>
  refreshRepo: () => Promise<unknown>
  /** Called after a delete is accepted, so the caller can clear a selection. */
  onDeleted?: (name: string) => void
}

/**
 * The "deletion in flight" state for a repository's archives.
 *
 * Deletion is asynchronous: the DELETE request only enqueues the borg job and
 * returns immediately, so without tracking the in-flight names a user can
 * re-trigger the same delete indefinitely before the first has even started.
 * Clearing a marker again is driven by three separate WebSocket events, which
 * is why this lives in one place rather than spread across the view.
 */
export interface UseArchiveDeletion {
  pending: Ref<ArchiveEntry | null>
  deleteLoading: Ref<boolean>
  isDeleting: (name: string) => boolean
  request: (archive: ArchiveEntry) => void
  close: () => void
  confirm: () => Promise<void>
  forget: (name: string) => void
  pruneToPresent: () => void
  sweepIdle: () => void
}

export function useArchiveDeletion(options: UseArchiveDeletionOptions): UseArchiveDeletion {
  const { success: toastSuccess, error: toastError } = useToast()

  const pending = ref<ArchiveEntry | null>(null)
  const deleteLoading = ref(false)
  const deletingNames = ref<Set<string>>(new Set())

  function isDeleting(name: string): boolean {
    return deletingNames.value.has(name)
  }

  function mark(name: string): void {
    deletingNames.value = new Set(deletingNames.value).add(name)
  }

  /** Drops a single marker, e.g. once ArchiveDeleted confirms it is gone. */
  function forget(name: string): void {
    if (!deletingNames.value.has(name)) return
    const next = new Set(deletingNames.value)
    next.delete(name)
    deletingNames.value = next
  }

  function request(archive: ArchiveEntry): void {
    if (isDeleting(archive.name)) return
    pending.value = archive
  }

  function close(): void {
    if (!deleteLoading.value) pending.value = null
  }

  async function confirm(): Promise<void> {
    const archive = pending.value
    if (!archive) return
    deleteLoading.value = true
    // Mark it as deleting before the request even goes out, not after it
    // resolves. On a fast repo the DELETE's DataChanged notification can
    // reach the WebSocket handler - and prune this archive from the list -
    // before the await below would otherwise return, which would mean the
    // "deleting" state was never observed and the row just vanishes instead
    // of showing the in-flight state the UI promises. The DELETE call itself
    // can also take a moment on its own (repo-level lock contention with
    // another queued operation, network latency), so the button must show
    // "in flight" the instant the user confirms either way.
    mark(archive.name)
    try {
      await options.deleteArchiveByName(archive)
      pending.value = null
      options.onDeleted?.(archive.name)
      await options.refreshRepo()
      toastSuccess('Archive deletion started. It will disappear once borg finishes.')
    } catch (e: unknown) {
      // The request never made it (or the server rejected it), so it was
      // never actually queued - undo the optimistic mark.
      forget(archive.name)
      toastError(extractError(e))
    } finally {
      deleteLoading.value = false
    }
  }

  /**
   * Drop markers for archives that are no longer in the list at all. Driven by
   * DataChanged: a successful delete is already handled by ArchiveDeleted, so
   * what is left here is a delete that failed and left the archive in place.
   */
  function pruneToPresent(): void {
    const stillPresent = new Set(options.sortedArchives.value.map((a) => a.name))
    const next = new Set([...deletingNames.value].filter((name) => stillPresent.has(name)))
    if (next.size !== deletingNames.value.size) deletingNames.value = next
  }

  /**
   * Clear the markers that were set when the repository's operation queue went
   * idle. Reloads first: by the time it returns the delete has concluded, so
   * the list is authoritative either way - present means genuinely stale or
   * failed, absent means already gone - and clearing is always correct. Clears
   * even if the reload fails, so a marker can never get stuck forever.
   *
   * Only the names marked *at the moment the event arrived* are swept, not
   * whatever the set holds once the refetch resolves: the user can start an
   * unrelated delete while that refetch is in flight, and clearing
   * unconditionally would wipe its just-set marker too.
   */
  function sweepIdle(): void {
    const toSweep = new Set(deletingNames.value)
    // Every op-idle transition fires this event (backups, prunes, rescans,
    // not just deletes), so skip the refetch entirely when there is nothing
    // to sweep rather than reloading the archive list for no reason.
    if (toSweep.size === 0) return
    options
      .reloadArchives(true)
      .catch(logger.error)
      .finally(() => {
        const next = new Set([...deletingNames.value].filter((name) => !toSweep.has(name)))
        if (next.size !== deletingNames.value.size) deletingNames.value = next
      })
  }

  return {
    pending,
    deleteLoading,
    isDeleting,
    request,
    close,
    confirm,
    forget,
    pruneToPresent,
    sweepIdle,
  }
}
