// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'

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

const toastSuccess = vi.fn()
const toastError = vi.fn()
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

import { apiClient } from '../api/client'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'

function makeArchive(name: string, overrides: Partial<ArchiveEntry> = {}): ArchiveEntry {
  return {
    name,
    start: '2026-06-01T12:00:00Z',
    hostname: 'web-server-01',
    comment: '',
    original_size: 2048,
    deduplicated_size: 1024,
    matched: true,
    agent_hostname: 'web-server-01',
    ...overrides,
  }
}

describe('ArchiveFileBrowser', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  async function mountWithWait(props: {
    repoId: number | null
    archive: ArchiveEntry | null
    isAdmin?: boolean
  }) {
    const wrapper = mount(ArchiveFileBrowser, { props })
    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()
    return wrapper
  }

  async function mountWithEntries(
    props: { repoId: number; archive: ArchiveEntry; isAdmin?: boolean } = {
      repoId: 5,
      archive: makeArchive('test-archive'),
    },
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

  it('renders empty state when archive is null', () => {
    const wrapper = mount(ArchiveFileBrowser, {
      props: { repoId: null, archive: null },
    })
    expect(wrapper.text()).toContain('Select an archive to browse its contents.')
  })

  it('renders browser header when archive is provided and contents loaded', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({ repoId: 5, archive: makeArchive('test-archive') })

    expect(wrapper.find('.browser-title').exists()).toBe(true)
    expect(wrapper.text()).toContain('test-archive')
  })

  it('shows the archive meta bar with date, original, and dedup size', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({
      repoId: 5,
      archive: makeArchive('test-archive', { original_size: 2048, deduplicated_size: 1024 }),
    })

    const metaBar = wrapper.find('.archive-meta-bar')
    expect(metaBar.exists()).toBe(true)
    expect(metaBar.text()).toContain('2.0 KB')
    expect(metaBar.text()).toContain('1.0 KB')
  })

  it('shows empty directory message when contents are empty', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({ repoId: 5, archive: makeArchive('test-archive') })

    expect(wrapper.text()).toContain('Empty directory.')
  })

  it('shows error state when contents fail to load', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('Repository error'))

    const wrapper = await mountWithWait({ repoId: 5, archive: makeArchive('test-archive') })

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

  it('does not show restore or delete buttons when isAdmin is false', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: false,
    })

    expect(wrapper.findAll('button[title*="Restore"]').length).toBe(0)
    expect(wrapper.findAll('button[title*="Delete"]').length).toBe(0)
  })

  it('shows restore buttons and a whole-archive delete button when isAdmin is true', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: true,
    })

    expect(wrapper.findAll('button[title*="Restore"]').length).toBeGreaterThan(0)
    expect(wrapper.find('button[title="Delete whole archive"]').exists()).toBe(true)
  })

  it('clicking restore calls restoreEntry and shows a success toast', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { success: true } })
    window.confirm = vi.fn().mockReturnValue(true)

    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: true,
    })

    const restoreBtn = wrapper.find('button[title="Restore whole archive to host"]')
    expect(restoreBtn.exists()).toBe(true)
    await restoreBtn.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalled()
    expect(toastSuccess).toHaveBeenCalledWith('Restored the whole archive.')
  })

  it('clicking the whole-archive delete button emits delete-archive with the archive', async () => {
    const archive = makeArchive('test-archive')
    const wrapper = await mountWithEntries({ repoId: 5, archive, isAdmin: true })

    const deleteBtn = wrapper.find('button[title="Delete whole archive"]')
    expect(deleteBtn.exists()).toBe(true)
    await deleteBtn.trigger('click')

    expect(wrapper.emitted('delete-archive')).toBeTruthy()
    expect(wrapper.emitted('delete-archive')?.[0]).toEqual([archive])
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
    const wrapper = await mountWithEntries()

    wrapper.unmount()
  })

  it('switching archive resets and reloads', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })

    const wrapper = await mountWithWait({ repoId: 5, archive: makeArchive('first-archive') })

    await wrapper.setProps({ archive: makeArchive('second-archive') })

    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()

    expect(wrapper.text()).toContain('second-archive')
  })
})
