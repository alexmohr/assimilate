// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'
import { getAppendOnlyStatus } from '../api/appendOnly'
import { listRepoStats } from '../api/repos'
import { logger } from '../utils/logger'
import { archiveKey, type RepoArchive } from '../utils/repoArchive'
import { useKeepArchive } from './useKeepArchive'

export interface UseQueuedDeletionsOptions {
  /** The list on screen; its repositories are the ones that matter. */
  archives: () => readonly RepoArchive[]
  /** Reading a repository's queue is admin-only, as is deleting. */
  isAdmin: () => boolean
  /**
   * The repositories known to be append-only. A caller that already holds the
   * repository row passes them; without it the repository list is read once.
   */
  appendOnlyRepoIds?: () => readonly number[] | undefined
}

export interface UseQueuedDeletions {
  /** `archiveKey`s of archives waiting for their repository's maintenance. */
  queuedKeys: ComputedRef<string[]>
  isAppendOnly: (repoId: number) => boolean
  isQueued: (archive: Pick<RepoArchive, 'repo_id' | 'name'>) => boolean
  /** Records a deletion just queued, before the server list catches up. */
  markQueued: (archive: Pick<RepoArchive, 'repo_id' | 'name'>) => void
  keep: (archive: RepoArchive) => Promise<void>
  reload: () => Promise<void>
  keeping: Ref<boolean>
}

/**
 * Which archives are queued for deletion at the next maintenance run.
 *
 * Deleting on an append-only repository does not remove anything: the agents'
 * key cannot, so the deletion is queued and the archive stays in borg until
 * the server's maintenance key does it. The list therefore keeps showing the
 * archive, marked, with a way to take the request back.
 */
export function useQueuedDeletions(options: UseQueuedDeletionsOptions): UseQueuedDeletions {
  const appendOnlyIds = ref<ReadonlySet<number>>(new Set())
  const queued = ref<ReadonlyMap<number, ReadonlySet<string>>>(new Map())
  const { keeping, keep: keepQueued } = useKeepArchive()
  let statsLoaded = false
  // Each reload and each optimistic mark starts a new generation, and a reload
  // that finishes in an older one is dropped: its answer was read before
  // whatever came since, and applying it would put back what that undid.
  let generation = 0

  const repoIds = computed<number[]>(() => [...new Set(options.archives().map((a) => a.repo_id))])

  async function resolveAppendOnly(): Promise<ReadonlySet<number>> {
    const given = options.appendOnlyRepoIds?.()
    if (given !== undefined) return new Set(given)
    if (statsLoaded) return appendOnlyIds.value
    const stats = await listRepoStats()
    statsLoaded = true
    return new Set(stats.filter((r) => Boolean(r.append_only)).map((r) => r.id))
  }

  async function reload(): Promise<void> {
    if (!options.isAdmin() || repoIds.value.length === 0) return
    const mine = ++generation
    try {
      const ids = await resolveAppendOnly()
      const onScreen = repoIds.value.filter((repoId) => ids.has(repoId))
      const statuses = await Promise.all(onScreen.map((id) => getAppendOnlyStatus(id)))
      const next = new Map<number, ReadonlySet<string>>()
      onScreen.forEach((id, index) => {
        const status = statuses[index]
        if (status) next.set(id, new Set(status.pending_deletions.map((p) => p.archive_name)))
      })
      if (mine !== generation) return
      appendOnlyIds.value = ids
      queued.value = next
    } catch (e: unknown) {
      // The marking is an addition to the list: without it the archives still
      // show, just unmarked.
      logger.error(e)
    }
  }

  // The set of repositories on screen, not the list itself: the list is
  // rebuilt on every refetch and the queue does not change with it.
  watch(() => repoIds.value.join(','), reload, { immediate: true })
  watch(() => options.appendOnlyRepoIds?.()?.join(','), reload)

  function isQueued(archive: Pick<RepoArchive, 'repo_id' | 'name'>): boolean {
    return queued.value.get(archive.repo_id)?.has(archive.name) === true
  }

  function markQueued(archive: Pick<RepoArchive, 'repo_id' | 'name'>): void {
    generation++
    const next = new Map(queued.value)
    next.set(archive.repo_id, new Set(next.get(archive.repo_id)).add(archive.name))
    queued.value = next
  }

  function keep(archive: RepoArchive): Promise<void> {
    return keepQueued(archive.repo_id, archive.name, reload)
  }

  return {
    queuedKeys: computed(() =>
      options
        .archives()
        .filter((a) => isQueued(a))
        .map(archiveKey),
    ),
    isAppendOnly: (repoId) =>
      options.appendOnlyRepoIds?.()?.includes(repoId) ?? appendOnlyIds.value.has(repoId),
    isQueued,
    markQueued,
    keep,
    reload,
    keeping,
  }
}
