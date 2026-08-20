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

// The meta bar links the archive's host into the agent page. These tests mount
// the browser bare, without the router, so stub the link rather than pull a
// router plugin into every mount below.
vi.mock('./BaseHostLink.vue', () => ({
  default: {
    props: ['hostname'],
    template: '<a class="host-link">{{ hostname }}</a>',
  },
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
    props: { repoId: number; archive: ArchiveEntry; isAdmin?: boolean; deleting?: boolean } = {
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

  async function triggerWholeArchiveRestore(): Promise<void> {
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
    expect(wrapper.find('.path-crumbs').exists()).toBe(true)
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
    await triggerWholeArchiveRestore()

    expect(apiClient.post).toHaveBeenCalled()
    expect(toastSuccess).toHaveBeenCalledWith('Restored the whole archive.')
  })

  it('shows an error toast when restore fails', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { success: false, error_message: 'Restore failed: disk full' },
    })
    await triggerWholeArchiveRestore()

    expect(toastError).toHaveBeenCalledWith('Restore failed: disk full')
    expect(toastSuccess).not.toHaveBeenCalled()
  })

  it('puts the whole-archive actions in the header, not on a hidden table row', async () => {
    // Download, restore and delete used to be reachable only through the file
    // table's "." row, with delete transparent until the pointer hovered it.
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: true,
    })

    const header = wrapper.find('.browser-actions')
    expect(header.exists()).toBe(true)
    expect(header.find('button[title="Download whole archive"]').text()).toContain('Download')
    expect(header.find('button[title="Restore whole archive to host"]').text()).toContain('Restore')
    expect(header.find('button[title="Delete whole archive"]').text()).toContain('Delete')

    // The table's action column keeps per-entry download and restore only.
    const rowActions = wrapper.findAll('.td-action')
    for (const cell of rowActions) {
      expect(cell.find('button[title*="Delete"]').exists()).toBe(false)
    }
  })

  it('names the archive and its host beside the file list', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
    })

    expect(wrapper.find('.browser-title-name').text()).toBe('test-archive')
    expect(wrapper.find('.archive-meta-bar .host-link').text()).toBe('web-server-01')
  })

  it('offers a way back up that is disabled at the archive root', async () => {
    const wrapper = await mountWithEntries()

    const up = wrapper.find('.browser-up')
    expect(up.attributes('disabled')).toBeDefined()

    const subdirRow = wrapper.findAll('tr.clickable').find((r) => r.text().includes('subdir'))
    await subdirRow!.trigger('click')
    await flushPromises()
    await nextTick()

    expect(wrapper.find('.browser-up').attributes('disabled')).toBeUndefined()

    const callsBefore = vi.mocked(apiClient.get).mock.calls.length
    await wrapper.find('.browser-up').trigger('click')
    await flushPromises()
    await nextTick()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBe(callsBefore + 1)
    expect(wrapper.findAll('.crumb')).toHaveLength(1)
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

  it('disables the whole-archive delete button and blocks the emit while deleting is true', async () => {
    const archive = makeArchive('test-archive')
    const wrapper = await mountWithEntries({ repoId: 5, archive, isAdmin: true, deleting: true })

    const deleteBtn = wrapper.find('button[title="Deletion in progress"]')
    expect(deleteBtn.exists()).toBe(true)
    expect(deleteBtn.attributes('disabled')).toBeDefined()
    expect(wrapper.find('button[title="Delete whole archive"]').exists()).toBe(false)

    await deleteBtn.trigger('click')

    expect(wrapper.emitted('delete-archive')).toBeFalsy()
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

  it('clicking a sortable column header reorders rows by that field', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: {
        index_status: 'done',
        entries: [
          { type: '-', path: 'zebra.txt', size: 10, mtime: '2026-06-01T12:00:00Z', mode: '644' },
          { type: '-', path: 'apple.txt', size: 20, mtime: '2026-06-01T12:00:00Z', mode: '644' },
        ],
      },
    })
    const wrapper = mount(ArchiveFileBrowser, {
      props: { repoId: 5, archive: makeArchive('test-archive') },
    })
    await flushPromises()
    await nextTick()

    const nameHeader = wrapper.findAll('th').find((h) => h.text().includes('Name'))
    expect(nameHeader).toBeTruthy()

    // The initial render is already ascending by name, so the first click
    // (ascending) is a no-op and the second click (descending) is the one
    // that proves the header is actually wired up to sorting.
    await nameHeader!.trigger('click')
    await nextTick()
    await nameHeader!.trigger('click')
    await nextTick()

    const fileRows = wrapper
      .findAll('tbody tr')
      .map((r) => r.text())
      .filter((t) => t.includes('.txt'))
    expect(fileRows[0]).toContain('zebra.txt')
    expect(fileRows[1]).toContain('apple.txt')
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
    const wrapper = await mountWithWait({ repoId: 5, archive: makeArchive('test-archive') })
    wrapper.unmount()

    expect(clearIntervalSpy).toHaveBeenCalled()
    clearIntervalSpy.mockRestore()
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

  it('clicking a directory row navigates into it and breadcrumb navigates back', async () => {
    const wrapper = await mountWithEntries()
    const callCountBefore = vi.mocked(apiClient.get).mock.calls.length

    const subdirRow = wrapper.findAll('tr.clickable').find((r) => r.text().includes('subdir'))
    expect(subdirRow).toBeTruthy()
    await subdirRow!.trigger('click')
    await flushPromises()
    await nextTick()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBe(callCountBefore + 1)
    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith(
      expect.stringContaining('/contents'),
      expect.objectContaining({ params: { path: 'subdir' } }),
    )

    let crumbs = wrapper.findAll('.crumb')
    expect(crumbs.length).toBe(2)
    expect(crumbs[0].text()).toBe('~')
    expect(crumbs[1].text()).toBe('subdir')

    await crumbs[0].trigger('click')
    await flushPromises()
    await nextTick()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBe(callCountBefore + 2)
    crumbs = wrapper.findAll('.crumb')
    expect(crumbs.length).toBe(1)
    expect(crumbs[0].text()).toBe('~')
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
      props: { repoId: 5, archive: makeArchive('test-archive') },
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
      props: { repoId: 5, archive: makeArchive('test-archive') },
    })

    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()

    expect(wrapper.text()).toContain('Indexing archive contents')
  })
})
