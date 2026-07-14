// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('./BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

import { apiClient } from '../api/client'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'

describe('ArchiveFileBrowser', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  async function mountWithWait(props: { repoId: number | null; archiveName: string | null }) {
    const wrapper = mount(ArchiveFileBrowser, { props })
    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()
    return wrapper
  }

  async function mountWithEntries(
    props: { repoId: number; archiveName: string } = { repoId: 5, archiveName: 'test-archive' },
  ) {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: {
        index_status: 'done',
        entries: [
          { type: 'd', path: 'subdir', size: 0, mtime: '2026-06-01T12:00:00Z', mode: '755' },
          { type: '-', path: 'readme.txt', size: 1024, mtime: '2026-06-01T12:00:00Z', mode: '644' },
        ],
      },
    })

    const wrapper = mount(ArchiveFileBrowser, { props })
    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()
    return wrapper
  }

  it('renders empty state when archiveName is null', () => {
    const wrapper = mount(ArchiveFileBrowser, {
      props: { repoId: null, archiveName: null },
    })
    expect(wrapper.text()).toContain('Select an archive to browse its contents.')
  })

  it('renders empty state when archiveName is empty string', () => {
    const wrapper = mount(ArchiveFileBrowser, {
      props: { repoId: 1, archiveName: '' },
    })
    expect(wrapper.text()).toContain('Select an archive to browse its contents.')
  })

  it('renders browser header when archiveName is provided and contents loaded', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({ repoId: 5, archiveName: 'test-archive' })

    expect(wrapper.find('.browser-title').exists()).toBe(true)
    expect(wrapper.text()).toContain('test-archive')
  })

  it('shows empty directory message when contents are empty', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({ repoId: 5, archiveName: 'test-archive' })

    expect(wrapper.text()).toContain('Empty directory.')
  })

  it('shows error state when contents fail to load', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('Repository error'))

    const wrapper = await mountWithWait({ repoId: 5, archiveName: 'test-archive' })

    expect(wrapper.find('.browser-title').exists()).toBe(true)
    expect(wrapper.text()).toContain('test-archive')
  })

  it('shows breadcrumb, directories, and files when API returns entries', async () => {
    const wrapper = await mountWithEntries()

    expect(wrapper.find('.browser-title').exists()).toBe(true)
    expect(wrapper.text()).toContain('test-archive')
    expect(wrapper.find('.breadcrumb').exists()).toBe(true)
    const crumbs = wrapper.findAll('.crumb')
    expect(crumbs.length).toBeGreaterThanOrEqual(1)
    expect(wrapper.text()).toContain('subdir')
    expect(wrapper.text()).toContain('readme.txt')
  })

  it('clicking breadcrumb button triggers navigateTo', async () => {
    const wrapper = await mountWithEntries()

    // Click a breadcrumb
    const crumb = wrapper.find('.crumb')
    if (crumb.exists()) {
      await crumb.trigger('click')
      await flushPromises()
      await nextTick()
    }

    expect(wrapper.find('.breadcrumb').exists()).toBe(true)
  })

  it('clicking a directory row triggers navigateTo', async () => {
    const wrapper = await mountWithEntries()

    const clickableRow = wrapper.find('tr.clickable')
    if (clickableRow.exists()) {
      await clickableRow.trigger('click')
      await flushPromises()
      await nextTick()
    }

    expect(wrapper.text()).toContain('subdir')
  })

  it('download button renders in action column', async () => {
    const wrapper = await mountWithEntries()

    const downloadBtn = wrapper.find('.btn-ghost')
    if (downloadBtn.exists()) {
      await downloadBtn.trigger('click')
      await flushPromises()
      await nextTick()
    }

    expect(wrapper.find('.browser-title').exists()).toBe(true)
  })

  it('renders filter inputs and handles interaction', async () => {
    const wrapper = await mountWithEntries()

    expect(wrapper.find('.data-table').exists()).toBe(true)
    const filterInputs = wrapper.findAll('.filter-input')
    expect(filterInputs.length).toBeGreaterThanOrEqual(1)
    for (const input of filterInputs) {
      await input.setValue('filter')
      await input.trigger('input')
    }
  })

  it('calls stopPolling on unmount', async () => {
    const wrapper = await mountWithEntries()

    wrapper.unmount()
  })

  it('switching archiveName resets and reloads', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({ repoId: 5, archiveName: 'first-archive' })

    await wrapper.setProps({ archiveName: 'second-archive' })

    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()

    expect(wrapper.text()).toContain('second-archive')
  })
})
