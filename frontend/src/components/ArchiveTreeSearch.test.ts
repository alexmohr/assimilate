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
    // The match is marked in the name; the folder beside it is plain.
    expect(hits[0]!.findAll('mark').map((m) => m.text())).toEqual(['nginx'])
    expect(hits[1]!.findAll('mark').map((m) => m.text())).toEqual(['nginx'])
    expect(hits[1]!.find('.browser-row-title').text()).toBe('nginx.conf')
    expect(hits[1]!.get('[data-testid="reveal-hit"]').text()).toBe('/etc/nginx')
    expect(hits[1]!.text()).toContain('2.0 KB')
    expect(wrapper.text()).toContain('2 matches in this archive')
    expect(wrapper.text()).not.toContain('The first')
  })

  it('narrows the search with filter pills, sizes in MiB', async () => {
    vi.setSystemTime(new Date('2026-02-15T12:00:00Z'))
    const wrapper = mountSearch()
    const pill = (group: string, label: string) =>
      wrapper
        .get(`[role="group"][aria-label="${group}"]`)
        .findAll('button')
        .find((b) => b.text() === label)!

    await pill('Type', 'Files').trigger('click')
    await pill('Size', 'Over 10 MB').trigger('click')
    await pill('Modified', 'Last 30 days').trigger('click')
    expect(wrapper.emitted('update:active')).toEqual([[true]])
    await settle()

    expect(searchArchiveTree).toHaveBeenCalledTimes(1)
    expect(vi.mocked(searchArchiveTree).mock.calls.at(-1)?.[2]).toEqual({
      query: undefined,
      kind: 'file',
      minSize: 10 * 1024 * 1024,
      modifiedFrom: '2026-01-16',
      limit: 200,
    })
    expect(pill('Type', 'Files').attributes('aria-pressed')).toBe('true')
    expect(pill('Type', 'Any').attributes('aria-pressed')).toBe('false')

    await pill('Size', 'Over 1 GB').trigger('click')
    await settle()
    expect(vi.mocked(searchArchiveTree).mock.calls.at(-1)?.[2]).toMatchObject({
      minSize: 1024 * 1024 * 1024,
    })

    // Back to every pill's "Any", there is no search and the folder returns.
    await pill('Type', 'Any').trigger('click')
    await pill('Size', 'Any').trigger('click')
    await pill('Modified', 'Any time').trigger('click')
    await settle()
    expect(wrapper.emitted('update:active')?.at(-1)).toEqual([false])
    expect(wrapper.find('[data-testid="tree-search-results"]').exists()).toBe(false)
  })

  it('searches folders only through the Type pills', async () => {
    const wrapper = mountSearch()
    await wrapper
      .get('[role="group"][aria-label="Type"]')
      .findAll('button')
      .find((b) => b.text() === 'Folders')!
      .trigger('click')
    await settle()
    expect(vi.mocked(searchArchiveTree).mock.calls.at(-1)?.[2]).toMatchObject({ kind: 'dir' })
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

  it('a click on a row ticks a file match, opens a folder match, and ticks nothing when disabled', async () => {
    const wrapper = mountSearch()
    await wrapper.get('[data-testid="tree-search"]').setValue('nginx')
    await settle()

    let hits = wrapper.findAll('[data-testid="search-hit"]')
    await hits[1]!.trigger('click')
    expect(wrapper.emitted('toggle')).toEqual([[HITS[1]]])
    expect(wrapper.emitted('open')).toBeUndefined()

    await wrapper.setProps({ disabled: true })
    hits = wrapper.findAll('[data-testid="search-hit"]')
    await hits[1]!.trigger('click')
    expect(wrapper.emitted('toggle')).toHaveLength(1)

    // A folder still opens while picking is disabled, and the search clears.
    await hits[0]!.trigger('click')
    expect(wrapper.emitted('open')).toEqual([['etc/nginx']])
    expect(wrapper.get<HTMLInputElement>('[data-testid="tree-search"]').element.value).toBe('')
  })

  it('closing mid-search drops the pending search and gives the folder back', async () => {
    const onActive = vi.fn()
    const wrapper = mount(ArchiveTreeSearch, {
      props: {
        repoId: 5,
        archiveName: 'nightly',
        tickState: () => 'unchecked',
        'onUpdate:active': onActive,
      },
    })
    await wrapper.get('[data-testid="tree-search"]').setValue('nginx')
    expect(onActive.mock.calls).toEqual([[true]])

    wrapper.unmount()
    await settle()

    expect(searchArchiveTree).not.toHaveBeenCalled()
    expect(onActive.mock.calls).toEqual([[true], [false]])
  })

  it('closing with no search entered reports nothing', () => {
    const onActive = vi.fn()
    const wrapper = mount(ArchiveTreeSearch, {
      props: {
        repoId: 5,
        archiveName: 'nightly',
        tickState: () => 'unchecked',
        'onUpdate:active': onActive,
      },
    })
    wrapper.unmount()
    expect(onActive).not.toHaveBeenCalled()
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

    expect(wrapper.text()).toContain('The first 2 matches in this archive')
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
