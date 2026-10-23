// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, RouterLinkStub } from '@vue/test-utils'
import { defineComponent, h, nextTick } from 'vue'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { AxiosError, AxiosHeaders, CanceledError } from 'axios'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { RestoreRun } from '../types/generated'
import { makeRestoreRun } from '../test-utils/restoreRun'
import { makeDatabaseDump } from '../test-utils/databaseDump'

import { pushWs, resetWsMock } from '../test-utils/wsMock'
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

// The repository chip is a plain `RouterLink`, and these specs mount the
// browser bare, with no router installed. `RouterLinkStub` is what every other
// spec here stubs it with (`ArchiveSelector`, `ArchiveSelectorRow`).
const GLOBAL = { stubs: { RouterLink: RouterLinkStub, teleport: true } }

const toastSuccess = vi.fn()
const toastError = vi.fn()
const toastInfo = vi.fn()
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError, info: toastInfo }),
}))

vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

/**
 * Pushes `run`'s change as the server does, naming only its id and status,
 * with `run` as what the browser's re-read then returns.
 */
async function push(run: RestoreRun): Promise<void> {
  vi.mocked(apiClient.get).mockResolvedValue({ data: run })
  pushWs('RestoreRunChanged', { id: run.id, status: run.status })
  await flushPromises()
}

/** The whole-archive restore the server records, running on the archive's host. */
function wholeArchiveRestore(overrides: Partial<RestoreRun> = {}): RestoreRun {
  return makeRestoreRun({
    hostname: 'web-server-01',
    archive_name: 'test-archive',
    paths: [],
    target_path: '/',
    ...overrides,
  })
}

import { apiClient } from '../api/client'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'
import FileDiffDialog from './FileDiffDialog.vue'
import FileHistoryDialog from './FileHistoryDialog.vue'

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
    resetWsMock()
    vi.resetAllMocks()
  })

  async function mountWithWait(props: {
    repoId: number | null
    archive: ArchiveEntry | null
    repoName?: string
    isAdmin?: boolean
  }) {
    const wrapper = mount(ArchiveFileBrowser, {
      props,
      global: GLOBAL,
    })
    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()
    return wrapper
  }

  async function mountWithEntries(
    props: {
      repoId: number
      archive: ArchiveEntry
      repoName?: string
      isAdmin?: boolean
      deleting?: boolean
      archives?: ArchiveEntry[]
    } = {
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

    const wrapper = mount(ArchiveFileBrowser, {
      props,
      global: GLOBAL,
    })
    await flushPromises()
    await nextTick()
    await flushPromises()
    await nextTick()
    return wrapper
  }

  /** The agents the restore dialog offers; the archive's own host is among them. */
  const AGENTS = [
    { id: 1, hostname: 'web-server-01', domain: null, display_name: null, is_connected: true },
  ]

  /**
   * Answers the dialog's agent list from `/agents` and everything else - the
   * archive's contents - with whatever the test set up before.
   */
  function serveAgents(): void {
    const contents = vi.mocked(apiClient.get).getMockImplementation()
    vi.mocked(apiClient.get).mockImplementation((url: string, ...rest: unknown[]) =>
      url === '/agents'
        ? Promise.resolve({ data: AGENTS })
        : (contents?.(url, ...rest) ?? Promise.resolve({ data: {} })),
    )
  }

  async function submitRestoreDialog(wrapper: ReturnType<typeof mount>): Promise<void> {
    await flushPromises()
    await wrapper.get('.restore-dialog').element.closest('form')!.dispatchEvent(new Event('submit'))
    await flushPromises()
  }

  async function triggerWholeArchiveRestore(): Promise<ReturnType<typeof mount>> {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: true,
    })
    serveAgents()

    const restoreBtn = wrapper.find('button[title="Restore whole archive"]')
    expect(restoreBtn.exists()).toBe(true)
    await restoreBtn.trigger('click')
    await submitRestoreDialog(wrapper)
    return wrapper
  }

  describe('picking entries', () => {
    it('selects every entry of the folder, and clears them again', async () => {
      const wrapper = await mountWithEntries()

      await wrapper.get('[data-testid="pick-all"]').setValue(true)
      expect(wrapper.get('[data-testid="browser-selection"]').text()).toContain('2 selected')
      // A folder's size is what is inside it, which the listing does not say.
      expect(wrapper.get('[data-testid="browser-selection"]').text()).not.toContain('KB')

      await wrapper.get('[data-testid="browser-selection"] button').trigger('click')
      expect(wrapper.find('[data-testid="browser-selection"]').exists()).toBe(false)
    })

    it('keeps what was picked in one folder while browsing another', async () => {
      const wrapper = await mountWithEntries()
      const fileRow = wrapper.findAll('tr').find((r) => r.text().includes('readme.txt'))
      await fileRow!.find('[data-testid="pick-entry"]').setValue(true)

      vi.mocked(apiClient.get).mockResolvedValue({ data: { index_status: 'done', entries: [] } })
      const dirRow = wrapper.findAll('tr').find((r) => r.text().includes('subdir'))
      await dirRow!.trigger('click')
      await flushPromises()

      expect(wrapper.get('[data-testid="browser-selection"]').text()).toContain('1 selected')
    })

    it('offers no checkbox on the parent row, and select-all leaves it out', async () => {
      const wrapper = await mountWithEntries()
      vi.mocked(apiClient.get).mockResolvedValue({
        data: {
          index_status: 'done',
          entries: [
            { path: 'subdir/inner.txt', type: '-', size: 10, mtime: '2026-01-01T00:00:00' },
          ],
        },
      })
      const dirRow = wrapper.findAll('tr').find((r) => r.text().includes('subdir'))
      await dirRow!.trigger('click')
      await flushPromises()

      const parentRow = wrapper.findAll('tbody tr').find((r) => r.text().includes('..'))
      expect(parentRow).toBeDefined()
      expect(parentRow!.find('[data-testid="pick-entry"]').exists()).toBe(false)

      await wrapper.get('[data-testid="pick-all"]').setValue(true)
      expect(wrapper.get('[data-testid="browser-selection"]').text()).toContain('1 selected')
    })

    it('downloads several entries as one tar.lz4', async () => {
      vi.mocked(apiClient.post).mockResolvedValue({ data: new Blob(['tar']) })
      const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:x')
      const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
      const wrapper = await mountWithEntries()

      await wrapper.get('[data-testid="pick-all"]').setValue(true)
      const download = wrapper
        .findAll('[data-testid="browser-selection"] button')
        .find((b) => b.text() === 'Download')
      await download!.trigger('click')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith(
        '/repos/5/archives/test-archive/download',
        { paths: ['subdir', 'readme.txt'] },
        expect.objectContaining({ responseType: 'blob' }),
      )
      expect(createObjectURL).toHaveBeenCalled()
      expect(revokeObjectURL).toHaveBeenCalledWith('blob:x')
      createObjectURL.mockRestore()
      revokeObjectURL.mockRestore()
    })

    it('downloads one picked file as itself, without packing a tar', async () => {
      const wrapper = await mountWithEntries()
      const createElementSpy = vi.spyOn(document, 'createElement')
      const fileRow = wrapper.findAll('tr').find((r) => r.text().includes('readme.txt'))
      await fileRow!.find('[data-testid="pick-entry"]').setValue(true)

      await clickSelectionDownload(wrapper)
      await flushPromises()

      const anchor = createElementSpy.mock.results
        .filter((_, i) => createElementSpy.mock.calls[i][0] === 'a')
        .map((r) => r.value as HTMLAnchorElement)
        .at(-1)
      expect(anchor?.getAttribute('href')).toContain('/extract?path=readme.txt')
      expect(apiClient.post).not.toHaveBeenCalledWith(
        expect.stringContaining('/download'),
        expect.anything(),
        expect.anything(),
      )
      createElementSpy.mockRestore()
    })

    function clickSelectionDownload(wrapper: Awaited<ReturnType<typeof mountWithEntries>>) {
      const download = wrapper
        .findAll('[data-testid="browser-selection"] button')
        .find((b) => b.text() === 'Download')
      return download!.trigger('click')
    }

    // A failed blob request carries its JSON error body as a Blob; read as
    // a plain error it would only say "Request failed with status code 409".
    it("names the server's error and its id when the download fails", async () => {
      const body = new Blob([
        JSON.stringify({ error: 'archive already being downloaded', error_id: 'blob-err-1' }),
      ])
      vi.mocked(apiClient.post).mockRejectedValue(
        new AxiosError('Request failed with status code 409', 'ERR_BAD_REQUEST', undefined, null, {
          status: 409,
          statusText: 'Conflict',
          headers: {},
          config: { headers: new AxiosHeaders() },
          data: body,
        }),
      )
      const wrapper = await mountWithEntries()

      await wrapper.get('[data-testid="pick-all"]').setValue(true)
      await clickSelectionDownload(wrapper)
      await flushPromises()

      expect(toastError).toHaveBeenCalledWith(
        expect.stringMatching(/archive already being downloaded.*blob-err-1/),
      )
    })

    it('cancels a download in flight', async () => {
      let signal: AbortSignal | undefined
      vi.mocked(apiClient.post).mockImplementation(
        (_url, _body, config) =>
          new Promise((_resolve, reject) => {
            signal = config?.signal as AbortSignal
            signal.addEventListener('abort', () => reject(new CanceledError()))
          }),
      )
      const wrapper = await mountWithEntries()

      await wrapper.get('[data-testid="pick-all"]').setValue(true)
      await clickSelectionDownload(wrapper)
      await nextTick()
      await wrapper.get('[data-testid="cancel-download"]').trigger('click')
      await flushPromises()

      expect(signal?.aborted).toBe(true)
      expect(wrapper.find('[data-testid="cancel-download"]').exists()).toBe(false)
      expect(toastError).not.toHaveBeenCalled()
    })

    it('offers restoring the selection only to an admin', async () => {
      const wrapper = await mountWithEntries({
        repoId: 5,
        archive: makeArchive('test-archive'),
        isAdmin: false,
      })
      await wrapper.get('[data-testid="pick-all"]').setValue(true)

      expect(wrapper.find('[data-testid="restore-picked"]').exists()).toBe(false)
    })

    it('forgets the selection when another archive opens', async () => {
      const wrapper = await mountWithEntries()
      await wrapper.get('[data-testid="pick-all"]').setValue(true)

      await wrapper.setProps({ archive: makeArchive('other-archive') })
      await flushPromises()

      expect(wrapper.find('[data-testid="browser-selection"]').exists()).toBe(false)
    })

    it('keeps the selection when a refresh hands back the same archive', async () => {
      const wrapper = await mountWithEntries()
      await wrapper.get('[data-testid="pick-all"]').setValue(true)

      // A background refetch rebuilds the list, so the open archive comes
      // back as a new object with the same name in the same repository.
      await wrapper.setProps({ archive: makeArchive('test-archive') })
      await flushPromises()

      expect(wrapper.get('[data-testid="browser-selection"]').text()).toContain('2 selected')
    })

    it('forgets the selection when the same name opens in another repository', async () => {
      const wrapper = await mountWithEntries()
      await wrapper.get('[data-testid="pick-all"]').setValue(true)

      await wrapper.setProps({ repoId: 6, archive: makeArchive('test-archive') })
      await flushPromises()

      expect(wrapper.find('[data-testid="browser-selection"]').exists()).toBe(false)
    })
  })

  it('goes back to the archive list from the header', async () => {
    const wrapper = await mountWithEntries()
    await wrapper.get('.browser-back').trigger('click')
    expect(wrapper.emitted('back')).toHaveLength(1)
  })

  describe('comparing a file with another archive', () => {
    const current = makeArchive('test-archive', { start: '2026-06-02T12:00:00Z' })
    const earlier = makeArchive('earlier-archive', { start: '2026-06-01T12:00:00Z' })

    it('offers Compare on a regular file once there is another archive', async () => {
      const wrapper = await mountWithEntries({
        repoId: 5,
        archive: current,
        archives: [earlier, current],
      })

      const buttons = wrapper.findAll('[data-testid="compare-file"]')
      expect(buttons).toHaveLength(1)
      expect(buttons[0].attributes('aria-label')).toBe('Compare readme.txt with another archive')
    })

    it('has nothing to compare with when the archive is the only one', async () => {
      const wrapper = await mountWithEntries({ repoId: 5, archive: current, archives: [current] })

      expect(wrapper.find('[data-testid="compare-file"]').exists()).toBe(false)
    })

    it('offers a file its history even when the archive is the only one', async () => {
      const wrapper = await mountWithEntries({ repoId: 5, archive: current, archives: [current] })

      const buttons = wrapper.findAll('[data-testid="file-history"]')
      expect(buttons).toHaveLength(1)
      expect(buttons[0].attributes('aria-label')).toBe('History of readme.txt')
    })

    it('opens and closes a file history', async () => {
      const wrapper = await mountWithEntries({ repoId: 5, archive: current, archives: [current] })
      vi.mocked(apiClient.get).mockResolvedValue({
        data: { path: 'readme.txt', hostname: 'web-01', agent_online: false, entries: [] },
      })

      await wrapper.get('[data-testid="file-history"]').trigger('click')
      await flushPromises()

      expect(apiClient.get).toHaveBeenLastCalledWith(
        '/repos/5/archives/test-archive/file-history',
        { params: { path: 'readme.txt' } },
      )
      expect(wrapper.findComponent(FileHistoryDialog).exists()).toBe(true)
      wrapper.findComponent(FileHistoryDialog).vm.$emit('close')
      await flushPromises()
      expect(wrapper.findComponent(FileHistoryDialog).exists()).toBe(false)
    })

    /** Mounts with an earlier archive to compare against and clicks Compare. */
    async function openComparison(): ReturnType<typeof mountWithEntries> {
      const wrapper = await mountWithEntries({
        repoId: 5,
        archive: current,
        archives: [earlier, current],
      })
      vi.mocked(apiClient.get).mockResolvedValue({
        data: { status: 'text', added: 0, removed: 0, hunks: [] },
      })
      await wrapper.get('[data-testid="compare-file"]').trigger('click')
      await flushPromises()
      return wrapper
    }

    it('opens the comparison against the earlier archive', async () => {
      await openComparison()

      expect(apiClient.get).toHaveBeenLastCalledWith(
        '/repos/5/archives/test-archive/file-diff',
        expect.objectContaining({
          params: expect.objectContaining({ path: 'readme.txt', against: 'earlier-archive' }),
        }),
      )
    })

    it('closes the comparison when the dialog asks to', async () => {
      const wrapper = await openComparison()
      expect(wrapper.findComponent(FileDiffDialog).exists()).toBe(true)

      wrapper.findComponent(FileDiffDialog).vm.$emit('close')
      await nextTick()

      expect(wrapper.findComponent(FileDiffDialog).exists()).toBe(false)
    })
  })

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

  it('downloads one file from its own row, not the whole archive', async () => {
    // The header now carries the whole-archive Download, so `.btn-ghost`
    // resolves there first: the per-row action needs selecting explicitly or
    // it goes untested.
    const wrapper = await mountWithEntries()

    const createElementSpy = vi.spyOn(document, 'createElement')
    const fileRow = wrapper.findAll('tr').find((r) => r.text().includes('readme.txt'))
    expect(fileRow).toBeDefined()

    await fileRow!.find('.td-action button[title="Download"]').trigger('click')
    await flushPromises()

    const anchor = createElementSpy.mock.results
      .filter((_, i) => createElementSpy.mock.calls[i][0] === 'a')
      .map((r) => r.value as HTMLAnchorElement)
      .at(-1)
    expect(anchor?.getAttribute('href')).toContain('/extract?path=readme.txt')
    expect(anchor?.download).toBe('readme.txt')

    // `resetAllMocks` clears calls but leaves the spy installed, and the next
    // test spies on document.createElement too - restore it so its own anchor
    // count is its own.
    createElementSpy.mockRestore()
  })

  it('restores the entries ticked in the table', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { success: true } })

    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: true,
    })
    serveAgents()

    const fileRow = wrapper.findAll('tr').find((r) => r.text().includes('readme.txt'))
    await fileRow!.find('[data-testid="pick-entry"]').setValue(true)
    const bar = wrapper.get('[data-testid="browser-selection"]')
    expect(bar.text()).toContain('1 selected · 1.0 KB')
    expect(bar.text()).toContain('to their original paths on web-server-01')

    await bar.get('[data-testid="restore-picked"]').trigger('click')
    await submitRestoreDialog(wrapper)

    expect(apiClient.post).toHaveBeenCalledWith(
      expect.stringContaining('/restore'),
      expect.objectContaining({ paths: ['readme.txt'] }),
    )
  })

  it('download button renders in action column and triggers download', async () => {
    const wrapper = await mountWithEntries()

    const createElementSpy = vi.spyOn(document, 'createElement')
    const appendChildSpy = vi.spyOn(document.body, 'appendChild')
    const removeChildSpy = vi.spyOn(document.body, 'removeChild')
    const downloadBtn = wrapper.find('button[title="Download whole archive"]')
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

  it('restoring the whole archive follows it with toasts', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: wholeArchiveRestore() })
    await triggerWholeArchiveRestore()

    expect(apiClient.post).toHaveBeenCalled()
    // The restore runs in the background: one toast as it starts...
    expect(toastInfo).toHaveBeenCalledWith('Restoring the whole archive onto web-server-01...')
    expect(toastSuccess).not.toHaveBeenCalled()

    // ...and one once the agent reports it done.
    await push(wholeArchiveRestore({ status: 'success', files_restored: 12 }))
    expect(toastSuccess).toHaveBeenCalledWith('Restored the whole archive onto web-server-01.')
  })

  it('shows an error toast when restore fails', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: wholeArchiveRestore() })
    await triggerWholeArchiveRestore()

    await push(
      wholeArchiveRestore({ status: 'failed', error_message: 'Restore failed: disk full' }),
    )

    expect(toastError).toHaveBeenCalledWith(
      'Restoring the whole archive onto web-server-01 failed: Restore failed: disk full',
    )
    expect(toastSuccess).not.toHaveBeenCalled()
  })

  it('says why the restore cannot be recorded, in the dialog that asked for it', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('Agent web-server-01 not found'))
    const wrapper = await triggerWholeArchiveRestore()

    expect(wrapper.get('.restore-dialog .form-error').text()).toContain(
      'Agent web-server-01 not found',
    )
    expect(toastInfo).not.toHaveBeenCalled()
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
    expect(header.find('button[title="Restore whole archive"]').text()).toContain('Restore')
    expect(header.find('button[title="Delete whole archive"]').text()).toContain('Delete')

    // The table's action column keeps per-entry download and compare only.
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

  // Download, restore and - above all - Delete in this same header act on one
  // repository, and a schedule that copies into several writes an archive of
  // the same name into each of them.
  it('names the repository the open archive lives in', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      repoName: 'offsite-weekly',
      archive: makeArchive('test-archive'),
    })

    const link = wrapper.findComponent(RouterLinkStub)
    expect(link.text()).toBe('offsite-weekly')
    expect(link.props('to')).toBe('/repos/5')
  })

  it('leaves the repository chip off when the caller names no repository', async () => {
    const wrapper = await mountWithEntries({ repoId: 5, archive: makeArchive('test-archive') })
    expect(wrapper.find('.archive-meta-bar .repo-link').exists()).toBe(false)
  })

  it('names the host borg recorded when no agent claims the archive', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive', {
        matched: false,
        hostname: 'legacy-nas',
        agent_hostname: 'web-server-01',
      }),
    })

    // The agent hostname is meaningless on an unmatched archive - it is
    // whichever agent the name happened to resemble, not its origin.
    expect(wrapper.find('.archive-meta-bar .host-link').text()).toBe('legacy-nas')
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

  it('switching archive while on its databases opens the next one on its files', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('first-archive'),
      isAdmin: true,
    })
    vi.mocked(apiClient.get).mockResolvedValue({ data: { dumps: [makeDatabaseDump()] } })
    await wrapper.findAll('.browser-view button')[1]!.trigger('click')
    await flushPromises()
    expect(wrapper.find('.browser-title-label').text()).toBe('Databases')

    vi.mocked(apiClient.get).mockResolvedValue({
      data: { index_status: 'done', entries: [] },
    })
    await wrapper.setProps({ archive: makeArchive('second-archive') })
    await flushPromises()

    expect(wrapper.find('.browser-title-label').text()).toBe('Files')
    expect(wrapper.find('[data-testid="archive-database-dumps"]').exists()).toBe(false)
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
    const downloadBtn = wrapper.find('button[title="Download whole archive"]')
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
    // Indexing polls on a real interval: left mounted, it would keep asking
    // for the index status during the tests that follow.
    wrapper.unmount()
  })

  it('switches to the database dumps the archive holds, and back to its files', async () => {
    const wrapper = await mountWithEntries({
      repoId: 5,
      archive: makeArchive('test-archive'),
      isAdmin: true,
    })
    vi.mocked(apiClient.get).mockResolvedValue({ data: { dumps: [makeDatabaseDump()] } })

    const views = wrapper.findAll('.browser-view button')
    expect(views.map((b) => b.text())).toEqual(['Files', 'Databases'])
    // The dumps are read only once the view is opened.
    expect(apiClient.get).not.toHaveBeenCalledWith('/repos/5/archives/test-archive/databases')

    await views[1]!.trigger('click')
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/5/archives/test-archive/databases')
    expect(wrapper.find('.browser-title-label').text()).toBe('Databases')
    expect(wrapper.find('.browser-path').exists()).toBe(false)
    const dumps = wrapper.find('[data-testid="archive-database-dumps"]')
    expect(dumps.text()).toContain('orders')
    expect(dumps.text()).toContain('shop/orders-db')
    expect(dumps.find('button[title="Restore orders"]').exists()).toBe(true)

    await views[0]!.trigger('click')
    expect(wrapper.find('.browser-title-label').text()).toBe('Files')
    expect(wrapper.find('.browser-path').exists()).toBe(true)
  })

  it('leaves database restores to those who may restore', async () => {
    const wrapper = await mountWithEntries()
    vi.mocked(apiClient.get).mockResolvedValue({ data: { dumps: [makeDatabaseDump()] } })

    await wrapper.findAll('.browser-view button')[1]!.trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="archive-database-dumps"]').text()).toContain('orders')
    expect(wrapper.find('button[title="Restore orders"]').exists()).toBe(false)
  })

  it('offers no database view before a repository is resolved', async () => {
    const wrapper = await mountWithWait({ repoId: null, archive: makeArchive('test-archive') })

    expect(wrapper.find('.browser-view').exists()).toBe(false)
  })

  // Issue #198: each directory opened is a browser history entry, so Back
  // walks back up through the directories visited until the archive root and
  // only then leaves the page.
  describe('browser history', () => {
    // One listing for every directory: the browser itself picks each level's
    // children out by parent path.
    const TREE = {
      index_status: 'done',
      entries: [
        { type: 'd', path: 'subdir', size: 0, mtime: '2026-06-01T12:00:00Z', mode: '755' },
        { type: 'd', path: 'subdir/nested', size: 0, mtime: '2026-06-01T12:00:00Z', mode: '755' },
        { type: '-', path: 'readme.txt', size: 1024, mtime: '2026-06-01T12:00:00Z', mode: '644' },
      ],
    }

    function makeRouter(): Router {
      const Page = defineComponent({ render: () => h('div') })
      return createRouter({
        history: createMemoryHistory(),
        routes: [
          { path: '/', component: Page },
          { path: '/repos/:id', component: Page },
        ],
      })
    }

    async function settle(): Promise<void> {
      await flushPromises()
      await nextTick()
      await flushPromises()
      await nextTick()
    }

    async function mountAt(
      router: Router,
      url: string,
      props: Partial<{
        archive: ArchiveEntry | null
        pathQueryKey: string
        routeArchiveName: string | null
      }> = {},
    ) {
      vi.mocked(apiClient.get).mockResolvedValue({ data: TREE })
      await router.push('/')
      await router.push(url)
      const wrapper = mount(ArchiveFileBrowser, {
        props: { repoId: 5, archive: makeArchive('test-archive'), ...props },
        global: { ...GLOBAL, plugins: [router] },
      })
      await settle()
      return wrapper
    }

    function crumbs(wrapper: ReturnType<typeof mount>): string[] {
      return wrapper.findAll('.crumb').map((c) => c.text())
    }

    async function openDir(wrapper: ReturnType<typeof mount>, name: string): Promise<void> {
      const row = wrapper.findAll('tr.clickable').find((r) => r.text().includes(name))
      expect(row).toBeTruthy()
      await row!.trigger('click')
      await settle()
    }

    function lastContentsPath(): unknown {
      const calls = vi.mocked(apiClient.get).mock.calls
      const options = calls[calls.length - 1][1] as { params?: { path?: string } } | undefined
      return options?.params?.path
    }

    it('walks back up the visited directories, then leaves the page', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?tab=archives')

      await openDir(wrapper, 'subdir')
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives', path: '/subdir' })
      await openDir(wrapper, 'nested')
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives', path: '/subdir/nested' })
      expect(crumbs(wrapper)).toEqual(['~', 'subdir', 'nested'])

      router.back()
      await settle()
      expect(crumbs(wrapper)).toEqual(['~', 'subdir'])
      expect(lastContentsPath()).toBe('subdir')

      router.back()
      await settle()
      expect(crumbs(wrapper)).toEqual(['~'])
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })

      router.back()
      await settle()
      expect(router.currentRoute.value.fullPath).toBe('/')
    })

    it('Forward reopens the directory Back left', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5')
      await openDir(wrapper, 'subdir')

      router.back()
      await settle()
      expect(crumbs(wrapper)).toEqual(['~'])

      router.forward()
      await settle()
      expect(crumbs(wrapper)).toEqual(['~', 'subdir'])
    })

    it('the breadcrumb and Up button add history entries too', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?path=%2Fsubdir%2Fnested')

      await wrapper.find('button[aria-label="Up one level"]').trigger('click')
      await settle()
      expect(router.currentRoute.value.query).toEqual({ path: '/subdir' })

      await wrapper.findAll('.crumb')[0].trigger('click')
      await settle()
      expect(router.currentRoute.value.query).toEqual({})

      router.back()
      await settle()
      expect(crumbs(wrapper)).toEqual(['~', 'subdir'])
    })

    it('opens a deep link at the directory it names', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?tab=archives&path=%2Fsubdir')

      expect(vi.mocked(apiClient.get)).toHaveBeenCalledTimes(1)
      expect(lastContentsPath()).toBe('subdir')
      expect(crumbs(wrapper)).toEqual(['~', 'subdir'])
    })

    it('opens a deep link once the archive is picked, not before', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?path=%2Fsubdir', { archive: null })
      expect(vi.mocked(apiClient.get)).not.toHaveBeenCalled()
      expect(router.currentRoute.value.query).toEqual({ path: '/subdir' })

      await wrapper.setProps({ archive: makeArchive('test-archive') })
      await settle()
      expect(crumbs(wrapper)).toEqual(['~', 'subdir'])
    })

    it('opening another archive starts at its root and drops the old directory', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?tab=archives')
      await openDir(wrapper, 'subdir')

      await wrapper.setProps({ archive: makeArchive('second-archive') })
      await settle()
      expect(crumbs(wrapper)).toEqual(['~'])
      expect(lastContentsPath()).toBeUndefined()
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })
    })

    it('closing the archive drops its directory from the route', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?tab=archives&path=%2Fsubdir')

      await wrapper.setProps({ archive: null })
      await settle()
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })
    })

    it('ignores a route path that climbs out of the archive', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?path=%2Fsubdir%2F..%2F..%2Fetc')
      expect(crumbs(wrapper)).toEqual(['~'])
      expect(lastContentsPath()).toBeUndefined()
    })

    it('keeps its directory under its own key when told to', async () => {
      const router = makeRouter()
      const wrapper = await mountAt(router, '/repos/5?path=%2Fother', { pathQueryKey: 'path-5' })
      expect(crumbs(wrapper)).toEqual(['~'])

      await openDir(wrapper, 'subdir')
      expect(router.currentRoute.value.query).toEqual({ path: '/other', 'path-5': '/subdir' })
    })

    it('drops its directory when unmounted while the page stays', async () => {
      // Unmounted from a parent rather than through `wrapper.unmount()`, which
      // takes the whole app - and the router's current route - with it.
      const router = makeRouter()
      vi.mocked(apiClient.get).mockResolvedValue({ data: TREE })
      await router.push('/repos/5?tab=archives&path=%2Fsubdir')
      const Tabs = defineComponent({
        props: { show: { type: Boolean, required: true } },
        setup(tabProps) {
          return (): ReturnType<typeof h> | null =>
            tabProps.show
              ? h(ArchiveFileBrowser, { repoId: 5, archive: makeArchive('test-archive') })
              : null
        },
      })
      const wrapper = mount(Tabs, {
        props: { show: true },
        global: { ...GLOBAL, plugins: [router] },
      })
      await settle()
      expect(crumbs(wrapper)).toEqual(['~', 'subdir'])

      await wrapper.setProps({ show: false })
      await settle()
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })
    })

    it('leaves the route alone when unmounted by navigating away', async () => {
      const router = makeRouter()
      vi.mocked(apiClient.get).mockResolvedValue({ data: TREE })
      await router.push('/repos/5?path=%2Fsubdir')
      const Shown = defineComponent({
        setup() {
          return (): ReturnType<typeof h> | null =>
            router.currentRoute.value.path === '/repos/5'
              ? h(ArchiveFileBrowser, { repoId: 5, archive: makeArchive('test-archive') })
              : null
        },
      })
      mount(Shown, { global: { ...GLOBAL, plugins: [router] } })
      await settle()

      await router.push('/?path=%2Felsewhere')
      await settle()
      expect(router.currentRoute.value.path).toBe('/')
      expect(router.currentRoute.value.query).toEqual({ path: '/elsewhere' })
    })

    // When the explorer keeps the archive in the route too, it owns both keys
    // and the browser only reads the directory.
    describe('with the archive in the route', () => {
      it("opens at the route's directory once the route names this archive", async () => {
        const router = makeRouter()
        const wrapper = await mountAt(router, '/repos/5?browse=test-archive&path=%2Fsubdir', {
          routeArchiveName: 'test-archive',
        })
        expect(crumbs(wrapper)).toEqual(['~', 'subdir'])
      })

      it('opens at the root while the route still names another archive', async () => {
        const router = makeRouter()
        const wrapper = await mountAt(router, '/repos/5?browse=other&path=%2Fsubdir', {
          routeArchiveName: 'other',
        })
        expect(crumbs(wrapper)).toEqual(['~'])
        expect(lastContentsPath()).toBeUndefined()
      })

      it('follows a directory change only for the archive the route names', async () => {
        const router = makeRouter()
        const wrapper = await mountAt(router, '/repos/5?browse=other', {
          routeArchiveName: 'other',
        })

        await router.push('/repos/5?browse=other&path=%2Fsubdir')
        await settle()
        expect(crumbs(wrapper)).toEqual(['~'])
      })

      it('leaves the route to the explorer when another archive opens or it closes', async () => {
        const router = makeRouter()
        const wrapper = await mountAt(router, '/repos/5?browse=test-archive&path=%2Fsubdir', {
          routeArchiveName: 'test-archive',
        })

        await wrapper.setProps({ archive: makeArchive('second-archive') })
        await settle()
        expect(crumbs(wrapper)).toEqual(['~'])
        await wrapper.setProps({ archive: null })
        await settle()
        expect(router.currentRoute.value.fullPath).toBe(
          '/repos/5?browse=test-archive&path=%2Fsubdir',
        )
      })
    })
  })

  describe('folder totals and tri-state picks', () => {
    // subdir/
    //   a.txt (1 KB)
    //   inner/
    //     x.txt (2 KB)
    //     y.txt (2 KB)
    // readme.txt (1 KB)
    const LISTINGS: Record<string, { type: string; path: string; size: number }[]> = {
      '': [
        { type: 'd', path: 'subdir', size: 0 },
        { type: '-', path: 'readme.txt', size: 1024 },
      ],
      subdir: [
        { type: 'd', path: 'subdir/inner', size: 0 },
        { type: '-', path: 'subdir/a.txt', size: 1024 },
      ],
      'subdir/inner': [
        { type: '-', path: 'subdir/inner/x.txt', size: 2048 },
        { type: '-', path: 'subdir/inner/y.txt', size: 2048 },
      ],
    }
    const TOTALS: Record<string, { files: number; bytes: number }> = {
      subdir: { files: 3, bytes: 5120 },
      'subdir/inner': { files: 2, bytes: 4096 },
    }

    function serveTree(): void {
      vi.mocked(apiClient.get).mockImplementation((_url: string, options?: unknown) => {
        const path = (options as { params?: { path?: string } } | undefined)?.params?.path ?? ''
        const entries = (LISTINGS[path] ?? []).map((entry) => ({
          ...entry,
          mtime: '2026-06-01T12:00:00Z',
          mode: '644',
        }))
        return Promise.resolve({ data: { index_status: 'done', entries } })
      })
      vi.mocked(apiClient.post).mockImplementation((_url: string, body?: unknown) => {
        const paths = (body as { paths: string[] }).paths
        return Promise.resolve({
          data: {
            index_status: 'done',
            totals: paths.map((path) => ({ path, files: 0, bytes: 0, ...TOTALS[path] })),
          },
        })
      })
    }

    function makeRouter(): Router {
      const Page = defineComponent({ render: () => h('div') })
      return createRouter({
        history: createMemoryHistory(),
        routes: [{ path: '/', component: Page }],
      })
    }

    async function settle(): Promise<void> {
      await flushPromises()
      await nextTick()
      await flushPromises()
      await nextTick()
    }

    async function mountTree(router: Router = makeRouter(), url = '/') {
      serveTree()
      await router.push(url)
      const wrapper = mount(ArchiveFileBrowser, {
        props: { repoId: 5, archive: makeArchive('test-archive') },
        global: { ...GLOBAL, plugins: [router] },
      })
      await settle()
      return wrapper
    }

    function row(wrapper: ReturnType<typeof mount>, name: string) {
      const found = wrapper
        .findAll('tbody tr')
        .find((r) => r.find('.name-text').exists() && r.find('.name-text').text() === name)
      expect(found, `row ${name}`).toBeTruthy()
      return found!
    }

    function pick(wrapper: ReturnType<typeof mount>, name: string) {
      return row(wrapper, name).find<HTMLInputElement>('[data-testid="pick-entry"]')
    }

    async function toggle(wrapper: ReturnType<typeof mount>, name: string): Promise<void> {
      await pick(wrapper, name).trigger('change')
      await settle()
    }

    async function open(router: Router, path: string): Promise<void> {
      await router.push(`/?path=${encodeURIComponent(`/${path}`)}`)
      await settle()
    }

    function summary(wrapper: ReturnType<typeof mount>): string {
      return wrapper.get('.browser-selection-count').text()
    }

    it('shows what each folder holds, read from the content index', async () => {
      const wrapper = await mountTree()

      expect(apiClient.post).toHaveBeenCalledWith('/repos/5/archives/test-archive/tree-totals', {
        paths: ['subdir'],
      })
      expect(row(wrapper, 'subdir').get('[data-testid="folder-files"]').text()).toBe('3 files')
      expect(row(wrapper, 'subdir').get('.td-size').text()).toBe('5.0 KB')
      expect(row(wrapper, 'readme.txt').find('[data-testid="folder-files"]').exists()).toBe(false)
    })

    it('adds up a selection holding a folder', async () => {
      const wrapper = await mountTree()

      await toggle(wrapper, 'subdir')
      await toggle(wrapper, 'readme.txt')

      expect(summary(wrapper)).toBe('2 selected · 6.0 KB · 4 files')
    })

    it('shows only the count while a folder has not been added up', async () => {
      const wrapper = await mountTree()
      vi.mocked(apiClient.post).mockRejectedValue(new Error('index unavailable'))
      await toggle(wrapper, 'readme.txt')
      expect(summary(wrapper)).toBe('1 selected · 1.0 KB · 1 file')

      await wrapper.setProps({ archive: makeArchive('other-archive') })
      await settle()
      await toggle(wrapper, 'subdir')

      expect(summary(wrapper)).toBe('1 selected')
      expect(row(wrapper, 'subdir').get('.td-size').text()).toBe('-')
    })

    it('marks a folder partly picked when something inside it is', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)
      await open(router, 'subdir')

      await toggle(wrapper, 'a.txt')
      await open(router, '')

      expect(pick(wrapper, 'subdir').element.indeterminate).toBe(true)
      expect(pick(wrapper, 'subdir').element.checked).toBe(false)
      expect(wrapper.get<HTMLInputElement>('[data-testid="pick-all"]').element.indeterminate).toBe(
        true,
      )
    })

    it('ticks everything in a folder by ticking the folder', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)

      await toggle(wrapper, 'subdir')
      await open(router, 'subdir')

      expect(pick(wrapper, 'a.txt').element.checked).toBe(true)
      expect(pick(wrapper, 'inner').element.checked).toBe(true)
      expect(wrapper.get<HTMLInputElement>('[data-testid="pick-all"]').element.checked).toBe(true)
      expect(summary(wrapper)).toBe('1 selected · 5.0 KB · 3 files')
    })

    it('collapses a folder whose every entry is ticked into the folder', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)
      await open(router, 'subdir/inner')

      await toggle(wrapper, 'x.txt')
      await toggle(wrapper, 'y.txt')

      expect(summary(wrapper)).toBe('1 selected · 4.0 KB · 2 files')
    })

    it('unticks one entry of a ticked folder by picking everything else in it', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)
      await toggle(wrapper, 'subdir')
      await open(router, 'subdir/inner')

      await toggle(wrapper, 'x.txt')

      // subdir's listing was never shown, so it is loaded in full to split it.
      expect(apiClient.get).toHaveBeenCalledWith('/repos/5/archives/test-archive/contents', {
        params: { path: 'subdir', limit: 10_000 },
      })
      expect(pick(wrapper, 'x.txt').element.checked).toBe(false)
      expect(pick(wrapper, 'y.txt').element.checked).toBe(true)
      expect(summary(wrapper)).toBe('2 selected · 3.0 KB · 2 files')
    })

    it('unticks everything here, splitting the ticked folder above it', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)
      await toggle(wrapper, 'subdir')
      await open(router, 'subdir/inner')

      await wrapper.get('[data-testid="pick-all"]').trigger('change')
      await settle()

      expect(summary(wrapper)).toBe('1 selected · 1.0 KB · 1 file')
      await open(router, 'subdir')
      expect(pick(wrapper, 'a.txt').element.checked).toBe(true)
      expect(pick(wrapper, 'inner').element.checked).toBe(false)
    })

    it('says so when a folder is too large to split', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)
      await toggle(wrapper, 'subdir')
      await open(router, 'subdir/inner')
      const huge = Array.from({ length: 10_000 }, (_, i) => ({
        type: '-',
        path: `subdir/f${i}`,
        size: 1,
        mtime: '',
        mode: '',
      }))
      vi.mocked(apiClient.get).mockResolvedValue({ data: { index_status: 'done', entries: huge } })

      await toggle(wrapper, 'x.txt')

      expect(toastError).toHaveBeenCalledWith(expect.stringContaining('too many entries'))
      expect(pick(wrapper, 'x.txt').element.checked).toBe(true)
    })

    it('keeps a file picked in a folder too long to list in full a file', async () => {
      const full = Array.from({ length: 100 }, (_, i) => ({
        type: '-',
        path: `big/f${String(i).padStart(3, '0')}.log`,
        size: 10,
        mtime: '2026-06-01T12:00:00Z',
        mode: '644',
      }))
      vi.mocked(apiClient.post).mockResolvedValue({ data: { index_status: 'done', totals: [] } })
      vi.mocked(apiClient.get).mockResolvedValue({ data: { index_status: 'done', entries: full } })
      const router = makeRouter()
      await router.push('/?path=%2Fbig')
      const wrapper = mount(ArchiveFileBrowser, {
        props: { repoId: 5, archive: makeArchive('test-archive') },
        global: { ...GLOBAL, plugins: [router] },
      })
      await settle()

      await toggle(wrapper, 'f007.log')
      expect(summary(wrapper)).toBe('1 selected · 10.0 B · 1 file')

      const createElementSpy = vi.spyOn(document, 'createElement')
      await wrapper
        .findAll('[data-testid="browser-selection"] button')
        .find((b) => b.text() === 'Download')!
        .trigger('click')
      await settle()
      const anchor = createElementSpy.mock.results
        .filter((_, i) => createElementSpy.mock.calls[i]?.[0] === 'a')
        .map((r) => r.value as HTMLAnchorElement)
        .at(-1)
      expect(anchor?.getAttribute('href')).toContain('/extract?path=big%2Ff007.log')
      expect(apiClient.post).not.toHaveBeenCalledWith(
        expect.stringContaining('/download'),
        expect.anything(),
        expect.anything(),
      )
      createElementSpy.mockRestore()
    })

    it('reports a listing that cannot be loaded and keeps the selection', async () => {
      const router = makeRouter()
      const wrapper = await mountTree(router)
      await toggle(wrapper, 'subdir')
      await open(router, 'subdir/inner')
      vi.mocked(apiClient.get).mockRejectedValue(new Error('network down'))

      await toggle(wrapper, 'x.txt')

      expect(toastError).toHaveBeenCalledWith('network down')
      expect(summary(wrapper)).toBe('1 selected · 5.0 KB · 3 files')
    })
  })
})
