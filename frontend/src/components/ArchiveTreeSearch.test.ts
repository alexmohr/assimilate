// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import ArchiveTreeSearch from './ArchiveTreeSearch.vue'
import type { TickState } from '../utils/archiveSelection'

vi.mock('../api/archives', () => ({ searchArchiveTree: vi.fn() }))

import { searchArchiveTree } from '../api/archives'

const HITS = [
  { type: 'd', path: 'etc/nginx', size: 0, mtime: '', mode: '' },
  { type: '-', path: 'etc/nginx/nginx.conf', size: 2048, mtime: '2026-01-10T02:00:00', mode: '' },
]

function mountSearch(tickState: (path: string) => TickState = () => 'unchecked') {
  return mount(ArchiveTreeSearch, {
    props: { repoId: 5, archiveName: 'nightly', tickState },
  })
}

/** Lets the debounce run out and the search answer. */
async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(400)
  await flushPromises()
}

describe('ArchiveTreeSearch', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.mocked(searchArchiveTree).mockReset()
    vi.mocked(searchArchiveTree).mockResolvedValue({
      index_status: 'done',
      entries: HITS,
      truncated: false,
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('searches as you type, once typing pauses, and highlights the match', async () => {
    const wrapper = mountSearch()

    await wrapper.get('[data-testid="tree-search"]').setValue('ng')
    await wrapper.get('[data-testid="tree-search"]').setValue('NGINX')
    expect(wrapper.emitted('update:active')).toEqual([[true]])
    await settle()

    expect(searchArchiveTree).toHaveBeenCalledTimes(1)
    expect(vi.mocked(searchArchiveTree).mock.calls[0]?.[2]).toMatchObject({ query: 'NGINX' })
    const hits = wrapper.findAll('[data-testid="search-hit"]')
    expect(hits).toHaveLength(2)
    expect(hits[1]!.findAll('mark').map((m) => m.text())).toEqual(['nginx', 'nginx'])
    expect(hits[1]!.text()).toContain('2.0 KB')
  })

  it('narrows the search with filters, sizes in MiB', async () => {
    const wrapper = mountSearch()
    await wrapper.get('[data-testid="tree-search-filters"]').trigger('click')

    await wrapper
      .findAll('.segmented-option')
      .find((b) => b.text() === 'Files')!
      .trigger('click')
    await wrapper.get('[data-testid="tree-search-min-size"]').setValue('1.5')
    await wrapper.get('[data-testid="tree-search-max-size"]').setValue('4')
    await wrapper.get('[data-testid="tree-search-from"]').setValue('2026-01-01')
    await wrapper.get('[data-testid="tree-search-to"]').setValue('2026-01-31')
    await settle()

    expect(vi.mocked(searchArchiveTree).mock.calls.at(-1)?.[2]).toEqual({
      query: undefined,
      kind: 'file',
      minSize: 1572864,
      maxSize: 4194304,
      modifiedFrom: '2026-01-01',
      modifiedTo: '2026-01-31',
      limit: 200,
    })
    expect(wrapper.find('.filter-badge').exists()).toBe(true)

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Reset filters')!
      .trigger('click')
    await settle()
    expect(wrapper.emitted('update:active')?.at(-1)).toEqual([false])
    expect(wrapper.find('[data-testid="tree-search-results"]').exists()).toBe(false)
  })

  it('shows how the selection ticks each match and reports a click on one', async () => {
    const wrapper = mountSearch((path) => (path === 'etc/nginx' ? 'partial' : 'checked'))
    await wrapper.get('[data-testid="tree-search"]').setValue('nginx')
    await settle()

    const picks = wrapper.findAll<HTMLInputElement>('[data-testid="pick-hit"]')
    expect(picks[0]!.element.indeterminate).toBe(true)
    expect(picks[1]!.element.checked).toBe(true)
    await picks[1]!.trigger('change')

    expect(wrapper.emitted('toggle')).toEqual([[HITS[1]]])
  })

  it('opens a folder match, or the folder a file sits in, and clears the search', async () => {
    const wrapper = mountSearch()
    await wrapper.get('[data-testid="tree-search"]').setValue('nginx')
    await settle()

    const reveal = wrapper.findAll('[data-testid="reveal-hit"]')
    await reveal[1]!.trigger('click')
    await reveal[0]!.trigger('click')

    expect(wrapper.emitted('open')).toEqual([['etc/nginx'], ['etc/nginx']])
    expect(wrapper.get<HTMLInputElement>('[data-testid="tree-search"]').element.value).toBe('')
  })

  it('says when nothing matches, the archive is not indexed, or the search failed', async () => {
    const wrapper = mountSearch()
    const field = wrapper.get('[data-testid="tree-search"]')

    vi.mocked(searchArchiveTree).mockResolvedValueOnce({
      index_status: 'done',
      entries: [],
      truncated: false,
    })
    await field.setValue('nothing')
    await settle()
    expect(wrapper.text()).toContain('Nothing in this archive matches.')

    vi.mocked(searchArchiveTree).mockResolvedValueOnce({
      index_status: 'indexing',
      entries: [],
      truncated: false,
    })
    await field.setValue('still')
    await settle()
    expect(wrapper.text()).toContain('still being indexed')

    vi.mocked(searchArchiveTree).mockRejectedValueOnce(new Error('index unavailable'))
    await field.setValue('broken')
    await settle()
    expect(wrapper.get('.error-banner').text()).toBe('index unavailable')
  })

  it('says when there are more matches than it shows', async () => {
    vi.mocked(searchArchiveTree).mockResolvedValue({
      index_status: 'done',
      entries: HITS,
      truncated: true,
    })
    const wrapper = mountSearch()

    await wrapper.get('[data-testid="tree-search"]').setValue('nginx')
    await settle()

    expect(wrapper.text()).toContain('Showing the first 2 matches.')
  })

  it('starts empty again when another archive opens', async () => {
    const wrapper = mountSearch()
    await wrapper.get('[data-testid="tree-search"]').setValue('nginx')
    await settle()

    await wrapper.setProps({ archiveName: 'weekly' })
    await settle()

    expect(wrapper.get<HTMLInputElement>('[data-testid="tree-search"]').element.value).toBe('')
    expect(wrapper.find('[data-testid="tree-search-results"]').exists()).toBe(false)
  })
})
