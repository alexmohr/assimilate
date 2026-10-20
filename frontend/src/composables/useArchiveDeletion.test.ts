// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { ref } from 'vue'
import { useArchiveDeletion } from './useArchiveDeletion'
import type { RepoArchive } from '../utils/repoArchive'

const toastSuccess = vi.fn()
const toastError = vi.fn()

vi.mock('./useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

/** The repository these tests delete from unless they name another copy. */
const REPO = 20
/** Another repository holding an archive of the same name, as a schedule's second target does. */
const OTHER = 21

function archive(name: string, repoId: number = REPO): RepoArchive {
  return {
    repo_id: repoId,
    repo_name: `repo-${repoId}`,
    name,
    start: '2026-01-01T00:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 1,
    deduplicated_size: 1,
    matched: true,
    agent_hostname: 'web-01',
  }
}

function setup(overrides: Record<string, unknown> = {}) {
  const sortedArchives = ref<RepoArchive[]>([archive('one'), archive('two'), archive('one', OTHER)])
  const deleteArchiveByName = vi.fn().mockResolvedValue(undefined)
  const reloadArchives = vi.fn().mockResolvedValue(undefined)
  const refreshRepo = vi.fn().mockResolvedValue(undefined)
  const onDeleted = vi.fn()
  const deletion = useArchiveDeletion({
    sortedArchives,
    deleteArchiveByName,
    reloadArchives,
    refreshRepo,
    onDeleted,
    ...overrides,
  })
  return {
    deletion,
    sortedArchives,
    deleteArchiveByName,
    reloadArchives,
    refreshRepo,
    onDeleted,
  }
}

describe('useArchiveDeletion', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    toastSuccess.mockClear()
    toastError.mockClear()
  })

  it('opens the confirmation for the requested archive', () => {
    const { deletion } = setup()
    deletion.request(archive('one'))
    expect(deletion.pending.value?.name).toBe('one')
  })

  it('refuses to re-open the confirmation for an archive already being deleted', async () => {
    const { deletion } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()
    expect(deletion.isDeleting(archive('one'))).toBe(true)

    deletion.request(archive('one'))
    expect(deletion.pending.value).toBeNull()
  })

  it('marks the archive as deleting before the request resolves', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const { deletion } = setup({ deleteArchiveByName: vi.fn().mockReturnValue(gate) })

    deletion.request(archive('one'))
    const inFlight = deletion.confirm()
    // The DELETE only enqueues the borg job, so the row must show its
    // in-flight state from the moment the user confirms.
    expect(deletion.isDeleting(archive('one'))).toBe(true)
    expect(deletion.deleteLoading.value).toBe(true)

    release()
    await inFlight
    expect(deletion.deleteLoading.value).toBe(false)
  })

  it('closes the dialog, refreshes the repo and reports success', async () => {
    const { deletion, refreshRepo, onDeleted } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    expect(deletion.pending.value).toBeNull()
    expect(refreshRepo).toHaveBeenCalledOnce()
    expect(onDeleted).toHaveBeenCalledWith(archive('one'))
    expect(toastSuccess).toHaveBeenCalledOnce()
  })

  it('undoes the optimistic mark when the delete request fails', async () => {
    const { deletion } = setup({
      deleteArchiveByName: vi.fn().mockRejectedValue(new Error('repo locked')),
    })
    deletion.request(archive('one'))
    await deletion.confirm()

    expect(deletion.isDeleting(archive('one'))).toBe(false)
    expect(toastError).toHaveBeenCalledOnce()
    expect(toastSuccess).not.toHaveBeenCalled()
  })

  it('will not close the dialog while the delete is in flight', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const { deletion } = setup({ deleteArchiveByName: vi.fn().mockReturnValue(gate) })

    deletion.request(archive('one'))
    const inFlight = deletion.confirm()
    deletion.close()
    expect(deletion.pending.value?.name).toBe('one')

    release()
    await inFlight
  })

  it('drops a single marker when the server confirms that archive is gone', async () => {
    const { deletion } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.forget('one', REPO)
    expect(deletion.isDeleting(archive('one'))).toBe(false)
  })

  it('prunes markers for archives that have left the list', async () => {
    const { deletion, sortedArchives } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    sortedArchives.value = [archive('two')]
    deletion.pruneToPresent()
    expect(deletion.isDeleting(archive('one'))).toBe(false)
  })

  it('keeps markers for archives a failed delete left in place', async () => {
    const { deletion } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.pruneToPresent()
    expect(deletion.isDeleting(archive('one'))).toBe(true)
  })

  it('skips the refetch entirely when the op queue drains with nothing marked', () => {
    const { deletion, reloadArchives } = setup()
    deletion.sweepIdle(REPO)
    expect(reloadArchives).not.toHaveBeenCalled()
  })

  it('sweeps stale markers once the op queue drains', async () => {
    const { deletion, reloadArchives } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.sweepIdle(REPO)
    await Promise.resolve()
    await Promise.resolve()

    expect(reloadArchives).toHaveBeenCalledWith(true)
    expect(deletion.isDeleting(archive('one'))).toBe(false)
  })

  it('clears the sweep even when the reload fails, so no row stays stuck', async () => {
    const { deletion } = setup({ reloadArchives: vi.fn().mockRejectedValue(new Error('offline')) })
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.sweepIdle(REPO)
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(deletion.isDeleting(archive('one'))).toBe(false)
  })

  it('leaves a delete started during the sweep untouched', async () => {
    let release: () => void = () => {}
    const reload = new Promise<void>((resolve) => {
      release = resolve
    })
    const { deletion } = setup({ reloadArchives: vi.fn().mockReturnValue(reload) })

    deletion.request(archive('one'))
    await deletion.confirm()
    deletion.sweepIdle(REPO)

    // A second, unrelated delete starts while the sweep's refetch is in flight.
    deletion.request(archive('two'))
    await deletion.confirm()

    release()
    await reload
    await Promise.resolve()
    await Promise.resolve()

    expect(deletion.isDeleting(archive('one'))).toBe(false)
    expect(deletion.isDeleting(archive('two'))).toBe(true)
  })

  // A schedule writes an archive of the same name into every target it has,
  // and one list holds all of them, so a marker held by name alone would speak
  // for a copy the user never touched.
  describe('across repositories', () => {
    it('sends the delete for the copy that was picked', async () => {
      const { deletion, deleteArchiveByName } = setup()

      deletion.request(archive('one', OTHER))
      await deletion.confirm()

      expect(deleteArchiveByName).toHaveBeenCalledWith(archive('one', OTHER))
    })

    it('leaves the identically-named copy in another repository alone', async () => {
      const { deletion } = setup()

      deletion.request(archive('one'))
      await deletion.confirm()

      expect(deletion.isDeleting(archive('one'))).toBe(true)
      expect(deletion.isDeleting(archive('one', OTHER))).toBe(false)
      // And that copy can still be deleted on its own.
      deletion.request(archive('one', OTHER))
      expect(deletion.pending.value).toEqual(archive('one', OTHER))
    })

    it('drops only the marker of the copy the server names', async () => {
      const { deletion } = setup()
      deletion.request(archive('one'))
      await deletion.confirm()
      deletion.request(archive('one', OTHER))
      await deletion.confirm()

      deletion.forget('one', OTHER)

      expect(deletion.isDeleting(archive('one', OTHER))).toBe(false)
      expect(deletion.isDeleting(archive('one'))).toBe(true)
    })

    // The list is evidence about every repository it holds, but each copy
    // only about its own: the same name surviving elsewhere does not keep a
    // deleted copy's marker alive.
    it('prunes each repository against its own copies', async () => {
      const { deletion, sortedArchives } = setup()
      deletion.request(archive('one'))
      await deletion.confirm()

      sortedArchives.value = [archive('two'), archive('one', OTHER)]
      deletion.pruneToPresent()

      expect(deletion.isDeleting(archive('one'))).toBe(false)
    })

    // The idle event names the repository whose queue drained; a delete still
    // running in another repository is not over just because this one is.
    it('sweeps the repository the idle event names, and no other', async () => {
      const { deletion } = setup()
      deletion.request(archive('one'))
      await deletion.confirm()
      deletion.request(archive('one', OTHER))
      await deletion.confirm()

      deletion.sweepIdle(OTHER)
      await Promise.resolve()
      await Promise.resolve()

      expect(deletion.isDeleting(archive('one', OTHER))).toBe(false)
      expect(deletion.isDeleting(archive('one'))).toBe(true)
    })
  })
})
