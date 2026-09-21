// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { ref } from 'vue'
import { useArchiveDeletion } from './useArchiveDeletion'
import type { ArchiveEntry } from './useArchiveBrowser'

const toastSuccess = vi.fn()
const toastError = vi.fn()

vi.mock('./useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

function archive(name: string): ArchiveEntry {
  return {
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
  const sortedArchives = ref<ArchiveEntry[]>([archive('one'), archive('two')])
  // Mutable, because the schedule Backups tab moves one explorer between a
  // schedule's target repositories rather than mounting a fresh one per repo.
  const repoId = ref<number | null>(20)
  const deleteArchiveByName = vi.fn().mockResolvedValue(undefined)
  const reloadArchives = vi.fn().mockResolvedValue(undefined)
  const refreshRepo = vi.fn().mockResolvedValue(undefined)
  const onDeleted = vi.fn()
  const deletion = useArchiveDeletion({
    sortedArchives,
    repoId: () => repoId.value,
    deleteArchiveByName,
    reloadArchives,
    refreshRepo,
    onDeleted,
    ...overrides,
  })
  return {
    deletion,
    sortedArchives,
    repoId,
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
    expect(deletion.isDeleting('one')).toBe(true)

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
    expect(deletion.isDeleting('one')).toBe(true)
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
    expect(onDeleted).toHaveBeenCalledWith('one')
    expect(toastSuccess).toHaveBeenCalledOnce()
  })

  it('undoes the optimistic mark when the delete request fails', async () => {
    const { deletion } = setup({
      deleteArchiveByName: vi.fn().mockRejectedValue(new Error('repo locked')),
    })
    deletion.request(archive('one'))
    await deletion.confirm()

    expect(deletion.isDeleting('one')).toBe(false)
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

    deletion.forget('one')
    expect(deletion.isDeleting('one')).toBe(false)
  })

  it('prunes markers for archives that have left the list', async () => {
    const { deletion, sortedArchives } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    sortedArchives.value = [archive('two')]
    deletion.pruneToPresent()
    expect(deletion.isDeleting('one')).toBe(false)
  })

  it('keeps markers for archives a failed delete left in place', async () => {
    const { deletion } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.pruneToPresent()
    expect(deletion.isDeleting('one')).toBe(true)
  })

  it('skips the refetch entirely when the op queue drains with nothing marked', () => {
    const { deletion, reloadArchives } = setup()
    deletion.sweepIdle()
    expect(reloadArchives).not.toHaveBeenCalled()
  })

  it('sweeps stale markers once the op queue drains', async () => {
    const { deletion, reloadArchives } = setup()
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.sweepIdle()
    await Promise.resolve()
    await Promise.resolve()

    expect(reloadArchives).toHaveBeenCalledWith(true)
    expect(deletion.isDeleting('one')).toBe(false)
  })

  it('clears the sweep even when the reload fails, so no row stays stuck', async () => {
    const { deletion } = setup({ reloadArchives: vi.fn().mockRejectedValue(new Error('offline')) })
    deletion.request(archive('one'))
    await deletion.confirm()

    deletion.sweepIdle()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(deletion.isDeleting('one')).toBe(false)
  })

  it('leaves a delete started during the sweep untouched', async () => {
    let release: () => void = () => {}
    const reload = new Promise<void>((resolve) => {
      release = resolve
    })
    const { deletion } = setup({ reloadArchives: vi.fn().mockReturnValue(reload) })

    deletion.request(archive('one'))
    await deletion.confirm()
    deletion.sweepIdle()

    // A second, unrelated delete starts while the sweep's refetch is in flight.
    deletion.request(archive('two'))
    await deletion.confirm()

    release()
    await reload
    await Promise.resolve()
    await Promise.resolve()

    expect(deletion.isDeleting('one')).toBe(false)
    expect(deletion.isDeleting('two')).toBe(true)
  })

  // A schedule writes an archive of the same name into every target it has, so
  // once one explorer can be moved between those targets a marker held by name
  // alone speaks for a copy the user never touched.
  describe('across repositories', () => {
    it('leaves the identically-named copy in another repository alone', async () => {
      const { deletion, repoId } = setup()

      deletion.request(archive('one'))
      await deletion.confirm()
      expect(deletion.isDeleting('one')).toBe(true)

      repoId.value = 21

      expect(deletion.isDeleting('one')).toBe(false)
    })

    it('still holds the marker when the same repository comes back', async () => {
      const { deletion, repoId } = setup()

      deletion.request(archive('one'))
      await deletion.confirm()
      repoId.value = 21
      repoId.value = 20

      // Dropping it here would let the user re-trigger a delete that is still
      // running, which is the whole reason these markers exist.
      expect(deletion.isDeleting('one')).toBe(true)
    })

    // `sortedArchives` is the browsed repository's list, so it is evidence
    // about that repository only.
    it('prunes only the browsed repository markers', async () => {
      const { deletion, repoId, sortedArchives } = setup()

      deletion.request(archive('one'))
      await deletion.confirm()

      repoId.value = 21
      sortedArchives.value = []
      deletion.pruneToPresent()

      repoId.value = 20
      expect(deletion.isDeleting('one')).toBe(true)
    })

    it('sweeps only the browsed repository markers when its queue goes idle', async () => {
      const { deletion, repoId } = setup()

      deletion.request(archive('one'))
      await deletion.confirm()

      repoId.value = 21
      deletion.request(archive('one'))
      await deletion.confirm()
      deletion.sweepIdle()
      await Promise.resolve()
      await Promise.resolve()

      expect(deletion.isDeleting('one')).toBe(false)
      repoId.value = 20
      expect(deletion.isDeleting('one')).toBe(true)
    })
  })
})
