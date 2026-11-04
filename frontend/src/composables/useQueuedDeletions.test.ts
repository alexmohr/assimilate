// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, ref } from 'vue'
import { flushPromises } from '@vue/test-utils'

vi.mock('../api/appendOnly', () => ({ getAppendOnlyStatus: vi.fn(), keepArchive: vi.fn() }))
vi.mock('../api/repos', () => ({ listRepoStats: vi.fn() }))

const toastSuccess = vi.fn()
const toastError = vi.fn()
vi.mock('./useToast', () => ({ useToast: () => ({ success: toastSuccess, error: toastError }) }))

import { getAppendOnlyStatus, keepArchive } from '../api/appendOnly'
import { listRepoStats } from '../api/repos'
import { appendOnlyStatusFixture } from '../test-utils/appendOnlyFixtures'
import { appendOnly, repoFixture } from '../test-utils/repoFixtures'
import { useQueuedDeletions } from './useQueuedDeletions'
import type { RepoArchive } from '../utils/repoArchive'

function archive(repoId: number, name: string): RepoArchive {
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

const queuedStatus = appendOnlyStatusFixture({
  pending_deletions: [
    {
      archive_name: 'old',
      reason: 'manual',
      requested_by: 'admin',
      requested_at: '2026-10-25T10:00:00Z',
    },
  ],
})

function setup(options: { isAdmin?: boolean; appendOnlyRepoIds?: number[] | undefined } = {}) {
  const archives = ref<RepoArchive[]>([archive(1, 'old'), archive(1, 'new'), archive(2, 'old')])
  const scope = effectScope()
  const queued = scope.run(() =>
    useQueuedDeletions({
      archives: () => archives.value,
      isAdmin: () => options.isAdmin ?? true,
      appendOnlyRepoIds: () => options.appendOnlyRepoIds,
    }),
  )!
  return { archives, queued }
}

describe('useQueuedDeletions', () => {
  beforeEach(() => {
    vi.mocked(getAppendOnlyStatus).mockReset().mockResolvedValue(queuedStatus)
    vi.mocked(keepArchive).mockReset().mockResolvedValue(undefined)
    vi.mocked(listRepoStats)
      .mockReset()
      .mockResolvedValue([
        repoFixture({ id: 1, append_only: appendOnly() }),
        repoFixture({ id: 2 }),
      ])
    toastSuccess.mockReset()
    toastError.mockReset()
  })

  it('finds the append-only repositories itself and marks what is queued there', async () => {
    const { queued } = setup()
    await flushPromises()

    expect(getAppendOnlyStatus).toHaveBeenCalledTimes(1)
    expect(getAppendOnlyStatus).toHaveBeenCalledWith(1)
    expect(queued.isAppendOnly(1)).toBe(true)
    expect(queued.isAppendOnly(2)).toBe(false)
    expect(queued.queuedKeys.value).toEqual(['1/old'])
    expect(queued.isQueued({ repo_id: 2, name: 'old' })).toBe(false)
  })

  it('uses the repositories the caller already knows', async () => {
    const { queued } = setup({ appendOnlyRepoIds: [2] })
    await flushPromises()

    expect(listRepoStats).not.toHaveBeenCalled()
    expect(getAppendOnlyStatus).toHaveBeenCalledWith(2)
    expect(queued.isAppendOnly(2)).toBe(true)
    expect(queued.isAppendOnly(1)).toBe(false)
  })

  it('asks nothing of a viewer who may not delete', async () => {
    setup({ isAdmin: false })
    await flushPromises()

    expect(listRepoStats).not.toHaveBeenCalled()
    expect(getAppendOnlyStatus).not.toHaveBeenCalled()
  })

  it('leaves the list unmarked when the lookup fails', async () => {
    vi.mocked(getAppendOnlyStatus).mockRejectedValue(new Error('boom'))
    const { queued } = setup()
    await flushPromises()

    expect(queued.queuedKeys.value).toEqual([])
  })

  it('marks a deletion just queued before the server list catches up', async () => {
    const { queued } = setup()
    await flushPromises()

    queued.markQueued({ repo_id: 1, name: 'new' })
    expect(queued.queuedKeys.value).toEqual(['1/old', '1/new'])
  })

  it('keeps an archive and refreshes the queue', async () => {
    const { queued } = setup()
    await flushPromises()
    vi.mocked(getAppendOnlyStatus).mockResolvedValue(appendOnlyStatusFixture())

    await queued.keep(archive(1, 'old'))

    expect(keepArchive).toHaveBeenCalledWith(1, 'old')
    expect(queued.queuedKeys.value).toEqual([])
    expect(toastSuccess).toHaveBeenCalled()
  })

  it('drops the answer of a reload that a newer one overtook', async () => {
    const { queued } = setup()
    await flushPromises()
    expect(queued.queuedKeys.value).toEqual(['1/old'])

    let answerOld!: (status: ReturnType<typeof appendOnlyStatusFixture>) => void
    vi.mocked(getAppendOnlyStatus).mockReturnValueOnce(
      new Promise((resolve) => {
        answerOld = resolve
      }),
    )
    const slow = queued.reload()
    vi.mocked(getAppendOnlyStatus).mockResolvedValueOnce(appendOnlyStatusFixture())
    await queued.reload()
    expect(queued.queuedKeys.value).toEqual([])

    // The older answer, read while the archive was still queued, arrives last.
    answerOld(queuedStatus)
    await slow
    expect(queued.queuedKeys.value).toEqual([])
  })

  it('keeps a deletion just queued when an older reload answers afterwards', async () => {
    vi.mocked(getAppendOnlyStatus).mockResolvedValue(appendOnlyStatusFixture())
    const { queued } = setup()
    await flushPromises()

    let answer!: (status: ReturnType<typeof appendOnlyStatusFixture>) => void
    vi.mocked(getAppendOnlyStatus).mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve
      }),
    )
    const inFlight = queued.reload()
    queued.markQueued({ repo_id: 1, name: 'new' })

    // It was read before the deletion was queued, so it does not know of it.
    answer(appendOnlyStatusFixture())
    await inFlight
    expect(queued.queuedKeys.value).toEqual(['1/new'])
  })

  it('reports a Keep that failed', async () => {
    const { queued } = setup()
    await flushPromises()
    vi.mocked(keepArchive).mockRejectedValue(new Error('gone'))

    await queued.keep(archive(1, 'old'))

    expect(toastError).toHaveBeenCalled()
    expect(queued.queuedKeys.value).toEqual(['1/old'])
  })
})
