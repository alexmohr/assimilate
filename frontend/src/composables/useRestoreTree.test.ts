// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import axios from 'axios'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

import { apiClient } from '../api/client'
import { logger } from '../utils/logger'
import { TREE_PAGE_SIZE, useRestoreTree, type EntryRow } from './useRestoreTree'
import type { ContentEntryResponse as ContentEntry } from '../types/generated'

function entry(path: string, type: string, size = 0, mtime = '2026-01-10T02:00:00'): ContentEntry {
  return { path, type, size, mtime, mode: '' }
}

/** Contents per archive and directory, as the server lists them. */
const CONTENTS: Record<string, Record<string, ContentEntry[]>> = {
  new: {
    '': [entry('var', 'd'), entry('etc', 'd')],
    etc: [entry('etc/hosts', '-', 120), entry('etc/nginx', 'd'), entry('etc/fstab', '-', 50)],
  },
  old: {
    etc: [entry('etc/hosts', '-', 100), entry('etc/fstab', '-', 50)],
  },
}

function mockServer(): void {
  vi.mocked(apiClient.get).mockImplementation((url: string, config?: unknown) => {
    const params = (config as { params?: Record<string, unknown> } | undefined)?.params ?? {}
    const archive = url.split('/')[4] ?? ''
    if (url.endsWith('/contents')) {
      const dir = typeof params.path === 'string' ? params.path : ''
      return Promise.resolve({
        data: { index_status: 'done', entries: CONTENTS[archive]?.[dir] ?? [] },
      } as never)
    }
    if (url.endsWith('/tree-search')) {
      return Promise.resolve({
        data: {
          index_status: 'done',
          entries: [entry('etc/nginx/conf.d/default.conf', '-', 7)],
          truncated: false,
        },
      } as never)
    }
    return Promise.reject(new Error(`unexpected GET ${url}`))
  })
  vi.mocked(apiClient.post).mockImplementation((url: string, body?: unknown) => {
    const paths = (body as { paths: string[] }).paths
    return Promise.resolve({
      data: {
        index_status: 'done',
        totals: paths.map((path) => ({ path, files: 3, bytes: 300 })),
      },
    } as never)
  })
}

function entryRows(rows: ReturnType<typeof useRestoreTree>['rows']['value']): EntryRow[] {
  return rows.filter((row): row is EntryRow => row.kind === 'entry')
}

/**
 * Opens an archive whose index is not done yet, then lets one status poll
 * answer with `status`. Runs under fake timers so the poll fires on demand.
 */
async function pollOnce(
  status: () => Promise<unknown>,
): Promise<ReturnType<typeof useRestoreTree>> {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url.endsWith('/index-status')) return status() as never
    return Promise.resolve({ data: { index_status: 'pending', entries: [] } } as never)
  })
  const tree = useRestoreTree()
  await tree.open({ repoId: 5, name: 'new' }, null)
  expect(tree.indexState.value).toBe('pending')
  await vi.advanceTimersByTimeAsync(2000)
  await flushPromises()
  return tree
}

/** The archive-relative directories the tree asked the server to list. */
function listedDirs(): unknown[] {
  return vi
    .mocked(apiClient.get)
    .mock.calls.filter(([url]) => url.endsWith('/contents'))
    .map(([, config]) => (config as { params: { path?: string } }).params.path ?? '')
}

describe('useRestoreTree', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockServer()
  })

  it('lists the root folders first and asks for their totals', async () => {
    const tree = useRestoreTree()

    await tree.open({ repoId: 5, name: 'new' }, null)

    expect(entryRows(tree.rows.value).map((row) => row.path)).toEqual(['etc', 'var'])
    expect(apiClient.post).toHaveBeenCalledWith('/repos/5/archives/new/tree-totals', {
      paths: ['etc', 'var'],
    })
    expect(tree.totals.value.etc).toEqual({ path: 'etc', files: 3, bytes: 300 })
    expect(tree.childrenOf('')).toEqual(['etc', 'var'])
  })

  it('opens a folder under its parent and compares its files with the previous archive', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, { repoId: 5, name: 'old' })

    await tree.toggleExpand('etc')

    const rows = entryRows(tree.rows.value)
    expect(rows.map((row) => [row.path, row.depth])).toEqual([
      ['etc', 0],
      ['etc/nginx', 1],
      ['etc/fstab', 1],
      ['etc/hosts', 1],
      ['var', 0],
    ])
    const byPath = Object.fromEntries(rows.map((row) => [row.path, row.entry]))
    expect(tree.changeOf(byPath['etc/hosts']!)).toBe('changed')
    expect(tree.changeOf(byPath['etc/fstab']!)).toBeNull()
    expect(tree.changeOf(byPath['etc/nginx']!)).toBeNull()
  })

  it('marks a file the previous archive did not have as new', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, { repoId: 5, name: 'old' })
    await tree.toggleExpand('etc')

    expect(tree.changeOf(entry('etc/motd', '-', 1))).toBe('new')
  })

  it('offers more entries of a directory that filled its page', async () => {
    const many = Array.from({ length: TREE_PAGE_SIZE }, (_, i) => entry(`f${i}`, '-', 1))
    CONTENTS.big = { '': many }
    const tree = useRestoreTree()

    await tree.open({ repoId: 5, name: 'big' }, null)

    const last = tree.rows.value[tree.rows.value.length - 1]
    expect(last?.kind).toBe('more')
    expect(tree.childrenOf('')).toBeNull()
    await tree.loadMore('')
    expect(apiClient.get).toHaveBeenLastCalledWith('/repos/5/archives/big/contents', {
      params: { limit: TREE_PAGE_SIZE * 2 },
    })
  })

  it('shows search matches with their folders opened and ticks them in place', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, null)

    await tree.search({ query: 'default' })

    const rows = entryRows(tree.rows.value)
    expect(rows.map((row) => [row.path, row.match, row.expanded])).toEqual([
      ['etc', false, true],
      ['etc/nginx', false, true],
      ['etc/nginx/conf.d', false, true],
      ['etc/nginx/conf.d/default.conf', true, false],
    ])
    expect(tree.searchActive.value).toBe(true)

    await tree.toggleExpand('etc/nginx')
    expect(entryRows(tree.rows.value).map((row) => row.path)).toEqual(['etc', 'etc/nginx'])

    await tree.search(null)
    expect(tree.searchActive.value).toBe(false)
  })

  it('polls while the archive is being indexed', async () => {
    vi.useFakeTimers()
    try {
      let indexed = false
      vi.mocked(apiClient.get).mockImplementation((url: string) => {
        if (url.endsWith('/index-status')) {
          indexed = true
          return Promise.resolve({ data: { status: 'done', file_count: 1, error: null } } as never)
        }
        return Promise.resolve({
          data: indexed
            ? { index_status: 'done', entries: [entry('etc', 'd')] }
            : { index_status: 'indexing', entries: [] },
        } as never)
      })
      const tree = useRestoreTree()

      await tree.open({ repoId: 5, name: 'new' }, null)
      expect(tree.indexState.value).toBe('indexing')

      await vi.advanceTimersByTimeAsync(2000)
      await flushPromises()

      expect(tree.indexState.value).toBe('done')
      expect(entryRows(tree.rows.value).map((row) => row.path)).toEqual(['etc'])
      tree.dispose()
    } finally {
      vi.useRealTimers()
    }
  })

  describe('while the index is not ready', () => {
    beforeEach(() => {
      vi.useFakeTimers()
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it.each([
      ['borg list failed', 'borg list failed'],
      [null, 'Archive indexing failed'],
    ])('reports a failed indexing job (error %s)', async (reported, shown) => {
      const tree = await pollOnce(() =>
        Promise.resolve({ data: { status: 'failed', file_count: 0, error: reported } }),
      )

      expect(tree.indexState.value).toBe('failed')
      expect(tree.error.value).toBe(shown)
      await vi.advanceTimersByTimeAsync(10_000)
      expect(
        vi.mocked(apiClient.get).mock.calls.filter(([url]) => url.endsWith('/index-status')),
      ).toHaveLength(1)
    })

    it('stops polling and shows why when the status cannot be read', async () => {
      const tree = await pollOnce(() => Promise.reject(new Error('server unreachable')))

      expect(tree.error.value).toBe('server unreachable')
      await vi.advanceTimersByTimeAsync(10_000)
      expect(
        vi.mocked(apiClient.get).mock.calls.filter(([url]) => url.endsWith('/index-status')),
      ).toHaveLength(1)
    })
  })

  it('shows why the archive could not be listed', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('archive is locked'))
    const tree = useRestoreTree()

    await tree.open({ repoId: 5, name: 'new' }, null)

    expect(tree.error.value).toBe('archive is locked')
    expect(tree.rootLoading.value).toBe(false)
    expect(tree.rows.value).toEqual([])
  })

  it('keeps the tree when folder totals or the previous archive cannot be read', async () => {
    const warn = vi.spyOn(logger, 'warn').mockImplementation(() => undefined)
    vi.mocked(apiClient.post).mockRejectedValue(new Error('totals unavailable'))
    vi.mocked(apiClient.get).mockImplementation((url: string, config?: unknown) => {
      if (url.includes('/archives/old/')) return Promise.reject(new Error('old archive gone'))
      const params = (config as { params?: { path?: string } }).params ?? {}
      return Promise.resolve({
        data: { index_status: 'done', entries: CONTENTS.new?.[params.path ?? ''] ?? [] },
      } as never)
    })
    const tree = useRestoreTree()

    await tree.open({ repoId: 5, name: 'new' }, { repoId: 5, name: 'old' })
    await tree.toggleExpand('etc')

    expect(entryRows(tree.rows.value).map((row) => row.path)).toContain('etc/hosts')
    expect(tree.totals.value).toEqual({})
    expect(tree.changeOf(entry('etc/hosts', '-', 120))).toBeNull()
    expect(warn).toHaveBeenCalledWith('could not load folder totals', expect.any(Error))
    expect(warn).toHaveBeenCalledWith(
      'could not compare with the previous archive',
      expect.any(Error),
    )
    warn.mockRestore()
  })

  it('closes an opened folder again', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, null)
    await tree.toggleExpand('etc')

    await tree.toggleExpand('etc')

    expect(entryRows(tree.rows.value).map((row) => [row.path, row.expanded])).toEqual([
      ['etc', false],
      ['var', false],
    ])
  })

  it('lists in full only the folders not completely listed yet', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, null)
    expect(tree.rootChildren.value).toEqual(['etc', 'var'])
    vi.mocked(apiClient.get).mockClear()

    await tree.ensureListings(['', 'etc'])

    expect(listedDirs()).toEqual(['etc'])
    expect(apiClient.get).toHaveBeenCalledWith('/repos/5/archives/new/contents', {
      params: { path: 'etc', limit: 100_000 },
    })
    expect(tree.childrenOf('etc')).toEqual(['etc/nginx', 'etc/fstab', 'etc/hosts'])
  })

  it.each([
    ['failed', 'This archive has no content index, so it cannot be searched.'],
    ['indexing', 'The archive is still being indexed. Search again in a moment.'],
  ])('explains why an archive whose index is %s cannot be searched', async (status, message) => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, null)
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: status, entries: [], truncated: false },
    } as never)

    await tree.search({ query: 'conf' })

    expect(tree.searchError.value).toBe(message)
    expect(tree.searchActive.value).toBe(true)
    expect(tree.searchCount.value).toBe(0)
    expect(tree.rows.value).toEqual([])
    expect(tree.searchLoading.value).toBe(false)
  })

  it('drops a search that a newer one replaced, without an error', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, null)
    vi.mocked(apiClient.get).mockImplementationOnce(
      (_url: string, config?: unknown) =>
        new Promise((_, reject) => {
          const { signal } = config as { signal: AbortSignal }
          signal.addEventListener('abort', () => reject(new axios.CanceledError()))
        }),
    )

    const first = tree.search({ query: 'old' })
    await tree.search({ query: 'default' })
    await first

    expect(tree.searchError.value).toBeNull()
    expect(tree.searchCount.value).toBe(1)
    expect(tree.searchLoading.value).toBe(false)
  })

  it('shows why a search failed', async () => {
    const tree = useRestoreTree()
    await tree.open({ repoId: 5, name: 'new' }, null)
    vi.mocked(apiClient.get).mockRejectedValue(new Error('search timed out'))

    await tree.search({ query: 'conf' })

    expect(tree.searchError.value).toBe('search timed out')
    expect(tree.searchActive.value).toBe(true)
    expect(tree.rows.value).toEqual([])
  })
})
