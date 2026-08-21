// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { useWebSocket } from './useWebSocket'
import { logger } from '../utils/logger'

/**
 * What a screen that browses archives has to expose for its "deleting..."
 * markers to be cleared again.
 *
 * `ArchiveExplorer` exposes exactly this, and a tab that wraps one forwards
 * the three calls to it.
 */
export interface ArchiveDeletionTarget {
  onArchiveDeleted: (name: string) => void
  onDataChanged: () => void
  onRepoIdle: () => void
}

export interface UseArchiveDeletionEventsOptions {
  /**
   * The mounted screen, or null while it is not rendered - archive tabs are
   * mounted lazily, so this is read per event rather than captured once.
   */
  target: () => ArchiveDeletionTarget | null | undefined
  /** The repository being browsed, or null before the caller has resolved one. */
  repoId: () => number | null
  /**
   * Silent refetch of the archive list, awaited before markers are pruned so
   * the prune runs against the list as it stands after the change, not before.
   */
  reload: () => Promise<unknown>
}

/**
 * Subscribes the three WebSocket events that clear an archive's "deletion in
 * flight" marker.
 *
 * Deleting is asynchronous - the DELETE only queues a borg job - so the row
 * stays marked until one of these says the job is over:
 *
 * - `ArchiveDeleted` names the archive that finished, which is the precise and
 *   synchronous case.
 * - `DataChanged` prunes markers for archives that are no longer in the list at
 *   all, after refetching it.
 * - `RepoOpChanged` going idle is the only one that covers a delete that was
 *   accepted and then *failed*: the archive is still there, so nothing else
 *   ever fires for it and the row would sit disabled forever.
 *
 * Every screen that can delete needs all three, which is why they live here
 * rather than being wired per view - the schedule's Backups tab and the
 * standalone Archives page both gained deletion without the failure half.
 */
export function useArchiveDeletionEvents(options: UseArchiveDeletionEventsOptions): void {
  const { onMessage } = useWebSocket()

  onMessage('ArchiveDeleted', (payload) => {
    if (payload.repo_id !== options.repoId()) return
    options.target()?.onArchiveDeleted(payload.archive_name)
  })

  onMessage('DataChanged', () => {
    // Silent: this runs on every DataChanged, not just user-triggered ones.
    // Blanking the list to a loading placeholder would hide the very row state
    // this refresh exists to reconcile.
    options
      .reload()
      .then(() => options.target()?.onDataChanged())
      .catch(logger.error)
  })

  onMessage('RepoOpChanged', (payload) => {
    if (payload.repo_id !== options.repoId()) return
    // Repository operations run strictly one at a time, so once the active one
    // is neither a delete nor the compact that follows one, every archive
    // delete queued for this repository has concluded - success or failure.
    // Anything still marked deleting at that point is stale.
    if (payload.op?.kind === 'delete_archive' || payload.op?.kind === 'compact_repo') return
    options.target()?.onRepoIdle()
  })
}
