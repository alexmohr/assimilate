// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'
import { TOTALS_BATCH, useArchiveTreeTotals } from './useArchiveTreeTotals'

vi.mock('../api/archives', () => ({ getArchiveTreeTotals: vi.fn() }))

import { getArchiveTreeTotals } from '../api/archives'

function answer(paths: readonly string[], status = 'done') {
  return {
    index_status: status,
    totals: paths.map((path) => ({ path, files: 1, bytes: path.length })),
  }
}

describe('useArchiveTreeTotals', () => {
  beforeEach(() => {
    vi.mocked(getArchiveTreeTotals).mockReset()
    vi.mocked(getArchiveTreeTotals).mockImplementation(async (_repo, _archive, paths) =>
      answer(paths),
    )
  })

  it('asks once for each path and keeps what it learnt', async () => {
    const { totals, ensure } = useArchiveTreeTotals(
      () => 5,
      () => 'nightly',
    )

    await ensure(['etc', 'var', 'etc'])
    await ensure(['etc'])

    expect(getArchiveTreeTotals).toHaveBeenCalledTimes(1)
    expect(getArchiveTreeTotals).toHaveBeenCalledWith(5, 'nightly', ['etc', 'var'])
    expect(totals.value.get('var')).toEqual({ path: 'var', files: 1, bytes: 3 })
  })

  it('splits a long list into requests the server accepts', async () => {
    const { totals, ensure } = useArchiveTreeTotals(
      () => 5,
      () => 'nightly',
    )
    const paths = Array.from({ length: TOTALS_BATCH + 1 }, (_, i) => `f${i}`)

    await ensure(paths)

    expect(getArchiveTreeTotals).toHaveBeenCalledTimes(2)
    expect(totals.value.size).toBe(TOTALS_BATCH + 1)
  })

  it('asks nothing without an open archive', async () => {
    const { ensure } = useArchiveTreeTotals(
      () => null,
      () => null,
    )

    await ensure(['etc'])

    expect(getArchiveTreeTotals).not.toHaveBeenCalled()
  })

  it('asks again once an archive still being indexed is done', async () => {
    vi.mocked(getArchiveTreeTotals).mockResolvedValueOnce(answer([], 'indexing'))
    const { totals, ensure } = useArchiveTreeTotals(
      () => 5,
      () => 'nightly',
    )

    await ensure(['etc'])
    expect(totals.value.has('etc')).toBe(false)
    await ensure(['etc'])

    expect(getArchiveTreeTotals).toHaveBeenCalledTimes(2)
    expect(totals.value.has('etc')).toBe(true)
  })

  it('asks again after a failed request', async () => {
    vi.mocked(getArchiveTreeTotals).mockRejectedValueOnce(new Error('offline'))
    const { totals, ensure } = useArchiveTreeTotals(
      () => 5,
      () => 'nightly',
    )

    await ensure(['etc'])
    await ensure(['etc'])

    expect(totals.value.has('etc')).toBe(true)
  })

  it('forgets one archive when another opens, and drops a late answer about it', async () => {
    const archive = ref('nightly')
    let release: () => void = () => {}
    vi.mocked(getArchiveTreeTotals).mockImplementationOnce(
      (_repo, _archive, paths) =>
        new Promise((resolve) => {
          release = () => resolve(answer(paths))
        }),
    )
    const { totals, ensure } = useArchiveTreeTotals(
      () => 5,
      () => archive.value,
    )

    const late = ensure(['etc'])
    archive.value = 'weekly'
    await nextTick()
    release()
    await late

    expect(totals.value.size).toBe(0)
    await ensure(['etc'])
    expect(getArchiveTreeTotals).toHaveBeenLastCalledWith(5, 'weekly', ['etc'])
    expect(totals.value.has('etc')).toBe(true)
  })
})
