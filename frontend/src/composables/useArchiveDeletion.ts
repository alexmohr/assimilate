// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type ComputedRef, type Ref } from 'vue'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useToast } from './useToast'
import type { RepoArchive } from '../utils/repoArchive'

/** Shared empty set, so reading a repository with no markers allocates nothing. */
const EMPTY: ReadonlySet<string> = new Set()

export interface UseArchiveDeletionOptions {
  /**
   * The archive list as it currently stands, used to prune stale markers.
   *
   * It can span several repositories, so every entry says which one it is in:
   * a delete acts on one *copy* - a (repository, archive name) pair - and the
   * same name exists in every target a schedule writes into.
   */
  sortedArchives: Ref<RepoArchive[]> | ComputedRef<RepoArchive[]>
  /** Sends the delete for `archive` to the repository it names. */
  deleteArchiveByName: (archive: RepoArchive) => Promise<unknown>
  /** Silent reload, so the panel does not flash a loading placeholder. */
  reloadArchives: (silent: boolean) => Promise<unknown>
  refreshRepo: () => Promise<unknown>
  /** Called after a delete is accepted, so the caller can clear a selection. */
  onDeleted?: (archive: RepoArchive) => void
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
  pending: Ref<RepoArchive | null>
  deleteLoading: Ref<boolean>
  isDeleting: (archive: Pick<RepoArchive, 'repo_id' | 'name'>) => boolean
  request: (archive: RepoArchive) => void
  close: () => void
  confirm: () => Promise<void>
  forget: (name: string, repoId: number) => void
  pruneToPresent: () => void
  sweepIdle: (repoId: number) => void
}

export function useArchiveDeletion(options: UseArchiveDeletionOptions): UseArchiveDeletion {
  const { success: toastSuccess, error: toastError } = useToast()

  const pending = ref<RepoArchive | null>(null)
  const deleteLoading = ref(false)
  /**
   * In-flight names per repository. Keyed by repository because the same
   * archive name exists in every target a schedule writes into, and a marker
   * belongs to the one copy actually being deleted.
   */
  const deletingByRepo = ref<Map<string, Set<string>>>(new Map())

  function namesIn(repo: string): ReadonlySet<string> {
    return deletingByRepo.value.get(repo) ?? EMPTY
  }

  /** Replaces the map rather than mutating it, so the refs above stay reactive. */
  function writeNames(repo: string, names: Set<string>): void {
    const next = new Map(deletingByRepo.value)
    if (names.size === 0) next.delete(repo)
    else next.set(repo, names)
    deletingByRepo.value = next
  }

  function isDeleting(archive: Pick<RepoArchive, 'repo_id' | 'name'>): boolean {
    return namesIn(String(archive.repo_id)).has(archive.name)
  }

  function markIn(repo: string, name: string): void {
    writeNames(repo, new Set(namesIn(repo)).add(name))
  }

  function forgetIn(repo: string, name: string): void {
    const names = namesIn(repo)
    if (!names.has(name)) return
    const next = new Set(names)
    next.delete(name)
    writeNames(repo, next)
  }

  /** Drops a single marker, e.g. once ArchiveDeleted confirms it is gone. */
  function forget(name: string, repoId: number): void {
    forgetIn(String(repoId), name)
  }

  function request(archive: RepoArchive): void {
    if (isDeleting(archive)) return
    pending.value = archive
  }

  function close(): void {
    if (!deleteLoading.value) pending.value = null
  }

  async function confirm(): Promise<void> {
    const archive = pending.value
    if (!archive) return
    const repo = String(archive.repo_id)
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
    markIn(repo, archive.name)
    try {
      await options.deleteArchiveByName(archive)
      pending.value = null
      options.onDeleted?.(archive)
      await options.refreshRepo()
      toastSuccess('Archive deletion started. It will disappear once borg finishes.')
    } catch (e: unknown) {
      // The request never made it (or the server rejected it), so it was
      // never actually queued - undo the optimistic mark.
      forgetIn(repo, archive.name)
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
    if (deletingByRepo.value.size === 0) return
    const present = new Map<string, Set<string>>()
    for (const archive of options.sortedArchives.value) {
      const repo = String(archive.repo_id)
      const names = present.get(repo) ?? new Set<string>()
      names.add(archive.name)
      present.set(repo, names)
    }
    // Each repository against its own copies: the same name in another
    // repository says nothing about whether this one is still there.
    for (const [repo, names] of deletingByRepo.value) {
      const stillPresent = present.get(repo) ?? new Set<string>()
      const next = new Set([...names].filter((name) => stillPresent.has(name)))
      if (next.size !== names.size) writeNames(repo, next)
    }
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
  function sweepIdle(repoId: number): void {
    const repo = String(repoId)
    const toSweep = new Set(namesIn(repo))
    // Every op-idle transition fires this event (backups, prunes, rescans,
    // not just deletes), so skip the refetch entirely when there is nothing
    // to sweep rather than reloading the archive list for no reason.
    if (toSweep.size === 0) return
    options
      .reloadArchives(true)
      .catch(logger.error)
      .finally(() => {
        const names = namesIn(repo)
        const next = new Set([...names].filter((name) => !toSweep.has(name)))
        if (next.size !== names.size) writeNames(repo, next)
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
