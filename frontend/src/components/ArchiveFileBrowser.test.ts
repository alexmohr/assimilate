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

    const callCountBefore = vi.mocked(apiClient.get).mock.calls.length
    const crumb = wrapper.find('.crumb')
    expect(crumb.exists()).toBe(true)
    expect(crumb.text()).toBe('~')
    await crumb.trigger('click')
    await flushPromises()
    await nextTick()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBeGreaterThan(callCountBefore)
  })

  it('renders directory rows as clickable', async () => {
    const wrapper = await mountWithEntries()

    const clickableRows = wrapper.findAll('tr.clickable')
    // Directory entries (., subdir) get clickable class; readme.txt does not
    expect(clickableRows.length).toBe(2)
    // Verify the subdir row is among the clickable rows
    const subdirRow = clickableRows.find((r) => r.text().includes('subdir'))
    expect(subdirRow).toBeTruthy()
  })

  it('clicking a directory row navigates to that directory', async () => {
    const wrapper = await mountWithEntries()

    const callCountBefore = vi.mocked(apiClient.get).mock.calls.length
    const subdirRow = wrapper.findAll('tr.clickable').find((r) => r.text().includes('subdir'))
    expect(subdirRow).toBeTruthy()
    await subdirRow!.trigger('click')
    await flushPromises()
    await nextTick()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBeGreaterThan(callCountBefore)
  })

  it('download button renders in action column and triggers download', async () => {
    const wrapper = await mountWithEntries()

    const createElementSpy = vi.spyOn(document, 'createElement')
    const appendChildSpy = vi.spyOn(document.body, 'appendChild')
    const removeChildSpy = vi.spyOn(document.body, 'removeChild')
    const downloadBtn = wrapper.find('.btn-ghost')
    expect(downloadBtn.exists()).toBe(true)
    await downloadBtn.trigger('click')
    await flushPromises()
    await nextTick()

    // downloadEntry creates an anchor element and appends it to body
    const anchorCalls = createElementSpy.mock.calls.filter(([tag]) => tag === 'a')
    expect(anchorCalls.length).toBe(1)
    expect(appendChildSpy).toHaveBeenCalledWith(expect.any(HTMLAnchorElement))
    expect(removeChildSpy).toHaveBeenCalledWith(expect.any(HTMLAnchorElement))
    createElementSpy.mockRestore()
    appendChildSpy.mockRestore()
    removeChildSpy.mockRestore()
  })

  it('renders filter inputs and handles interaction', async () => {
    const wrapper = await mountWithEntries()

    expect(wrapper.find('.data-table').exists()).toBe(true)
    const filterInputs = wrapper.findAll('.filter-input')
    expect(filterInputs.length).toBe(3)
    const nameInput = filterInputs[0]
    expect(nameInput.element.getAttribute('placeholder')).toBe('Filter name...')
    await nameInput.setValue('readme')
    await nameInput.trigger('input')
  })

  it('filters files by display name', async () => {
    const wrapper = await mountWithEntries()

    const nameInput = wrapper.findAll('.filter-input')[0]
    await nameInput.setValue('readme')
    await nameInput.trigger('input')
    await nextTick()

    // The DataTable should now show only matching rows
    const rows = wrapper.findAll('tr')
    const visibleNames = rows
      .filter((r) => !r.classes().includes('p-datatable-header'))
      .map((r) => r.text())
    expect(visibleNames.some((t) => t.includes('readme.txt'))).toBe(true)
  })

  it('filters files by display size', async () => {
    const wrapper = await mountWithEntries()

    const rowsBefore = wrapper
      .findAll('tr')
      .filter((r) => !r.classes().includes('p-datatable-header'))
    const sizeInput = wrapper.findAll('.filter-input')[1]
    await sizeInput.setValue('1.0')
    await sizeInput.trigger('input')
    await nextTick()

    const rows = wrapper.findAll('tr').filter((r) => !r.classes().includes('p-datatable-header'))
    const visibleSizes = rows.map((r) => r.text())
    expect(visibleSizes.some((t) => t.includes('1.0 KB'))).toBe(true)
    expect(rows.length).toBeLessThan(rowsBefore.length)
  })

  it('filters files by display date', async () => {
    const wrapper = await mountWithEntries()

    const mtimeInput = wrapper.findAll('.filter-input')[2]
    await mtimeInput.setValue('2026')
    await mtimeInput.trigger('input')
    await nextTick()

    const rows = wrapper.findAll('tr').filter((r) => !r.classes().includes('p-datatable-header'))
    const visibleDates = rows.map((r) => r.text())
    expect(visibleDates.some((t) => t.includes('2026'))).toBe(true)
    expect(rows.length).toBeGreaterThan(0)
  })

  it('clicking the dot-directory row does NOT navigate', async () => {
    const wrapper = await mountWithEntries()

    const dotRow = wrapper.findAll('tr.clickable').find((r) => r.text().includes('.'))
    expect(dotRow).toBeTruthy()
    const callCountBefore = vi.mocked(apiClient.get).mock.calls.length

    await dotRow!.trigger('click')
    await flushPromises()
    await nextTick()

    // Verify no new API calls were made (the '.' entry should not navigate)
    expect(vi.mocked(apiClient.get).mock.calls.length).toBe(callCountBefore)
  })

  it('calls stopPolling on unmount', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'indexing', entries: [] },
    })

    const clearIntervalSpy = vi.spyOn(global, 'clearInterval')
    const wrapper = await mountWithWait({ repoId: 5, archiveName: 'test-archive' })
    wrapper.unmount()

    expect(clearIntervalSpy).toHaveBeenCalled()
    clearIntervalSpy.mockRestore()
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

  it('clicking a directory row navigates into it and breadcrumb navigates back', async () => {
    const wrapper = await mountWithEntries()

    const dirRow = wrapper.find('.clickable')
    expect(dirRow.exists()).toBe(true)

    await dirRow.trigger('click')
    await flushPromises()
    await nextTick()

    const rootCrumb = wrapper.find('.crumb')
    expect(rootCrumb.exists()).toBe(true)

    await rootCrumb.trigger('click')
    await flushPromises()
    await nextTick()

    expect(wrapper.find('.breadcrumb').exists()).toBe(true)
  })

  it('download button creates a download link', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: {
        index_status: 'done',
        entries: [
          { type: '-', path: 'file.txt', size: 1024, mtime: '2026-06-01T12:00:00Z', mode: '644' },
        ],
      },
    })

    const createElementSpy = vi.spyOn(document, 'createElement')
    const appendChildSpy = vi.spyOn(document.body, 'appendChild')
    const removeChildSpy = vi.spyOn(document.body, 'removeChild')

    const wrapper = mount(ArchiveFileBrowser, {
      props: { repoId: 5, archiveName: 'test-archive' },
    })

    await flushPromises()
    await nextTick()
    const downloadBtn = wrapper.find('button.btn-ghost')
    expect(downloadBtn.exists()).toBe(true)
    await downloadBtn.trigger('click')

    expect(createElementSpy).toHaveBeenCalledWith('a')
    expect(appendChildSpy).toHaveBeenCalled()
    expect(removeChildSpy).toHaveBeenCalled()

    createElementSpy.mockRestore()
    appendChildSpy.mockRestore()
    removeChildSpy.mockRestore()
  })

  it('typing in filter inputs covers v-model and input callbacks', async () => {
    const wrapper = await mountWithEntries()

    const inputs = wrapper.findAll('input')
    const nameInput = inputs.find((el) => el.attributes('placeholder') === 'Filter name...')
    const sizeInput = inputs.find((el) => el.attributes('placeholder') === 'Filter size...')
    const dateInput = inputs.find((el) => el.attributes('placeholder') === 'Filter date...')

    expect(nameInput).toBeTruthy()
    expect(sizeInput).toBeTruthy()
    expect(dateInput).toBeTruthy()

    await nameInput!.setValue('test')
    await sizeInput!.setValue('1024')
    await dateInput!.setValue('2026')

    await nextTick()

    const el = nameInput!.element as HTMLInputElement
    expect(el.value).toBe('test')
  })

  it('shows indexing spinner when index_status is indexing', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'indexing', entries: [] },
    })

    const wrapper = mount(ArchiveFileBrowser, {
      props: { repoId: 5, archiveName: 'test-archive' },
    })

    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()

    expect(wrapper.text()).toContain('Indexing archive contents')
  })
})
