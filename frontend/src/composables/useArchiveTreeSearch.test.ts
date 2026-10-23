// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'
import { CanceledError } from 'axios'
import { SEARCH_LIMIT, useArchiveTreeSearch } from './useArchiveTreeSearch'

vi.mock('../api/archives', () => ({ searchArchiveTree: vi.fn() }))

import { searchArchiveTree } from '../api/archives'

const HIT = { type: '-', path: 'etc/hosts', size: 10, mtime: '', mode: '' }

describe('useArchiveTreeSearch', () => {
  beforeEach(() => {
    vi.mocked(searchArchiveTree).mockReset()
    vi.mocked(searchArchiveTree).mockResolvedValue({
      index_status: 'done',
      entries: [HIT],
      truncated: true,
    })
  })

  it('searches the open archive and keeps its matches', async () => {
    const { entries, truncated, search } = useArchiveTreeSearch(
      () => 5,
      () => 'nightly',
    )

    await search({ query: 'hosts' })

    expect(searchArchiveTree).toHaveBeenCalledWith(
      5,
      'nightly',
      { query: 'hosts', limit: SEARCH_LIMIT },
      expect.any(AbortSignal),
    )
    expect(entries.value).toEqual([HIT])
    expect(truncated.value).toBe(true)
  })

  it('clears the results for no search, or without an archive', async () => {
    const archive = ref<string | null>('nightly')
    const { entries, search } = useArchiveTreeSearch(
      () => 5,
      () => archive.value,
    )
    await search({ query: 'hosts' })

    await search(null)
    expect(entries.value).toEqual([])

    archive.value = null
    await search({ query: 'hosts' })
    expect(searchArchiveTree).toHaveBeenCalledTimes(1)
  })

  it('says when the archive is not indexed yet', async () => {
    vi.mocked(searchArchiveTree).mockResolvedValue({
      index_status: 'indexing',
      entries: [],
      truncated: false,
    })
    const { notIndexed, search } = useArchiveTreeSearch(
      () => 5,
      () => 'nightly',
    )

    await search({ query: 'hosts' })

    expect(notIndexed.value).toBe(true)
  })

  it('reports a failed search, but not one it cancelled itself', async () => {
    const { error, search } = useArchiveTreeSearch(
      () => 5,
      () => 'nightly',
    )

    vi.mocked(searchArchiveTree).mockRejectedValueOnce(new Error('index unavailable'))
    await search({ query: 'hosts' })
    expect(error.value).toBe('index unavailable')

    vi.mocked(searchArchiveTree).mockRejectedValueOnce(new CanceledError())
    await search({ query: 'hosts' })
    expect(error.value).toBeNull()
  })

  it('answers only the latest search, and forgets results when another archive opens', async () => {
    const archive = ref('nightly')
    let releaseFirst: () => void = () => {}
    vi.mocked(searchArchiveTree).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          releaseFirst = () => resolve({ index_status: 'done', entries: [HIT], truncated: false })
        }),
    )
    const { entries, loading, search } = useArchiveTreeSearch(
      () => 5,
      () => archive.value,
    )

    const first = search({ query: 'old' })
    expect(loading.value).toBe(true)
    archive.value = 'weekly'
    await nextTick()
    releaseFirst()
    await first

    expect(entries.value).toEqual([])
    expect(loading.value).toBe(false)
  })
})
