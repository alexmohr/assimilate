// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { renderWithPlugins } from '../test-utils'
import type { RepoArchive } from '../utils/repoArchive'

const mockRequestArchiveDelete = vi.fn()

vi.mock('../composables/useArchiveBrowser', () => ({
  requestArchiveDelete: (repoId: number, name: string) => mockRequestArchiveDelete(repoId, name),
}))

vi.mock('./ArchiveFileBrowser.vue', () => ({
  default: {
    name: 'ArchiveFileBrowser',
    props: [
      'repoId',
      'repoName',
      'archive',
      'archives',
      'isAdmin',
      'deleting',
      'pathQueryKey',
      'routeArchiveName',
    ],
    emits: ['delete-archive'],
    template:
      '<div class="stub-browser"><button class="stub-delete" @click="$emit(\'delete-archive\', archive)" /></div>',
  },
}))

const toastSuccess = vi.fn()
const toastError = vi.fn()
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

import ArchiveExplorer from './ArchiveExplorer.vue'

/** The repository these mounts browse; the event handlers now name it. */
const REPO_ID = 7

const ARCHIVE: RepoArchive = {
  repo_id: REPO_ID,
  repo_name: 'server-daily',
  name: 'web-01-2026-03-01',
  start: '2026-03-01T02:00:00Z',
  hostname: 'web-01',
  comment: '',
  original_size: 1000,
  deduplicated_size: 100,
  matched: true,
  agent_hostname: 'web-01',
}

function explorer(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ArchiveExplorer, {
    props: {
      archives: [ARCHIVE],
      selected: null,
      isAdmin: true,
      ...props,
    },
  })
}

function confirmButton(): HTMLButtonElement | null {
  return document.body.querySelector<HTMLButtonElement>('.modal-dialog button.btn-danger')
}

describe('ArchiveExplorer', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockRequestArchiveDelete.mockResolvedValue(undefined)
  })

  it('hands the picked archive to the file browser', async () => {
    const wrapper = explorer()
    await wrapper.findAll('.archive-row-select')[0].trigger('click')

    expect(wrapper.emitted('update:selected')?.[0]).toEqual([ARCHIVE])
  })

  it("forwards the selector's count of archives no agent claims", async () => {
    // The repository tab draws its unmatched banner from this rather than
    // deriving "unmatched" a second time beside the list that already knows.
    const wrapper = explorer({
      archives: [
        ARCHIVE,
        { ...ARCHIVE, name: 'legacy-1', matched: false, agent_hostname: null, hostname: 'nas-01' },
        { ...ARCHIVE, name: 'legacy-2', matched: false, agent_hostname: null, hostname: 'nas-01' },
      ],
    })
    await flushPromises()

    const vm = wrapper.vm as unknown as { unmatchedCount: number; unmatchedHostnames: string[] }
    expect(vm.unmatchedCount).toBe(2)
    expect(vm.unmatchedHostnames).toEqual(['nas-01'])
  })

  it('offers deletion only to an admin', () => {
    expect(explorer().find('.archive-row-delete').exists()).toBe(true)
    expect(explorer({ isAdmin: false }).find('.archive-row-delete').exists()).toBe(false)
  })

  it('gates the browser on the same rule, so restore needs an admin too', () => {
    const browser = (props: Record<string, unknown>) =>
      explorer({ selected: ARCHIVE, ...props }).findComponent({ name: 'ArchiveFileBrowser' })

    expect(browser({}).props('isAdmin')).toBe(true)
    expect(browser({ isAdmin: false }).props('isAdmin')).toBe(false)
  })

  // One list can hold several repositories' archives; the open one is browsed,
  // compared and restored against its own.
  it('browses the open archive in its own repository', () => {
    const offsite: RepoArchive = { ...ARCHIVE, repo_id: 9, repo_name: 'offsite' }
    const browser = explorer({ archives: [ARCHIVE, offsite], selected: offsite }).findComponent({
      name: 'ArchiveFileBrowser',
    })

    expect(browser.props('repoId')).toBe(9)
    expect(browser.props('repoName')).toBe('offsite')
    expect(browser.props('archives')).toEqual([offsite])
  })

  it('swaps the list for the open archive on a phone, and back', async () => {
    const wrapper = explorer({ selected: ARCHIVE })
    expect(wrapper.find('.archive-browser-layout').classes()).toContain('layout-detail-open')

    await wrapper.setProps({ selected: null })
    expect(wrapper.find('.archive-browser-layout').classes()).not.toContain('layout-detail-open')
  })

  it("hands the browser the caller's directory query key, or none for the default", () => {
    const browser = (props: Record<string, unknown>) =>
      explorer({ selected: ARCHIVE, ...props }).findComponent({ name: 'ArchiveFileBrowser' })

    expect(browser({}).props('pathQueryKey')).toBeUndefined()
    expect(browser({ pathQueryKey: 'path-7' }).props('pathQueryKey')).toBe('path-7')
  })

  it('names the archive and the repository in the confirmation', async () => {
    const wrapper = explorer()
    await wrapper.find('.archive-row-delete').trigger('click')
    await flushPromises()

    const message = document.body.querySelector('.archive-delete-message')?.textContent ?? ''
    expect(message).toContain(ARCHIVE.name)
    expect(message).toContain('server-daily')

    wrapper.unmount()
  })

  it('queues the delete against the repository the archive belongs to', async () => {
    const wrapper = explorer()
    await wrapper.find('.archive-row-delete').trigger('click')
    await flushPromises()

    confirmButton()!.click()
    await flushPromises()

    expect(mockRequestArchiveDelete).toHaveBeenCalledWith(7, ARCHIVE.name)
    expect(toastSuccess).toHaveBeenCalled()
    expect(wrapper.find('.archive-row-pending').exists()).toBe(true)
  })

  it('rolls the in-flight marker back when the request is rejected', async () => {
    mockRequestArchiveDelete.mockRejectedValue(new Error('Connection refused'))

    const wrapper = explorer()
    await wrapper.find('.archive-row-delete').trigger('click')
    await flushPromises()
    confirmButton()!.click()
    await flushPromises()

    expect(toastError).toHaveBeenCalledWith('Connection refused')
    expect(wrapper.find('.archive-row-pending').exists()).toBe(false)
    expect(wrapper.find('button[title="Delete archive"]').exists()).toBe(true)
  })

  it('refreshes what the delete invalidated', async () => {
    const refreshAfterDelete = vi.fn().mockResolvedValue(undefined)
    const wrapper = explorer({ refreshAfterDelete })

    await wrapper.find('.archive-row-delete').trigger('click')
    await flushPromises()
    confirmButton()!.click()
    await flushPromises()

    expect(refreshAfterDelete).toHaveBeenCalled()
  })

  it('opens the same confirmation from the file browser as from the row', async () => {
    const wrapper = explorer({ selected: ARCHIVE })
    await wrapper.find('.stub-delete').trigger('click')
    await flushPromises()

    expect(document.body.querySelector('.archive-delete-message')?.textContent).toContain(
      ARCHIVE.name,
    )

    wrapper.unmount()
  })

  it('clears the marker when the server reports the archive gone', async () => {
    const wrapper = explorer()
    await wrapper.find('.archive-row-delete').trigger('click')
    await flushPromises()
    confirmButton()!.click()
    await flushPromises()
    expect(wrapper.find('.archive-row-pending').exists()).toBe(true)

    wrapper.vm.onArchiveDeleted(ARCHIVE.name, REPO_ID)
    await flushPromises()

    expect(wrapper.find('.archive-row-pending').exists()).toBe(false)
  })

  it('sweeps stale markers once the repository queue goes idle', async () => {
    const reload = vi.fn().mockResolvedValue(undefined)
    const wrapper = explorer({ reload })

    await wrapper.find('.archive-row-delete').trigger('click')
    await flushPromises()
    confirmButton()!.click()
    await flushPromises()

    wrapper.vm.onRepoIdle(REPO_ID)
    await flushPromises()

    expect(reload).toHaveBeenCalledWith(true)
    expect(wrapper.find('.archive-row-pending').exists()).toBe(false)
  })

  // The open archive lives in the route next to the browser's directory, so
  // picking one is a history entry and a link can name both.
  describe('route', () => {
    const SECOND: ArchiveEntry = { ...ARCHIVE, name: 'web-01-2026-03-02' }

    async function mountAt(url: string, props: Record<string, unknown> = {}) {
      // A router that has already navigated, as the app's is before any view
      // mounts: the harness's own starts navigating only as it mounts.
      const Page = defineComponent({ render: () => h('div') })
      const router = createRouter({
        history: createMemoryHistory(),
        routes: [
          { path: '/', component: Page },
          { path: '/repos/:id', component: Page },
        ],
      })
      await router.push('/')
      await router.push(url)
      const wrapper = mount(ArchiveExplorer, {
        props: {
          repoId: REPO_ID,
          repoName: 'server-daily',
          archives: [ARCHIVE, SECOND],
          selected: null,
          isAdmin: true,
          ...props,
        },
        global: { plugins: [router], stubs: { RouterLink: true, Teleport: true } },
      })
      await flushPromises()
      return { wrapper, router }
    }

    function selections(wrapper: ReturnType<typeof mount>): unknown[] {
      return (wrapper.emitted('update:selected') ?? []).map(
        ([a]) => (a as ArchiveEntry | null)?.name ?? null,
      )
    }

    it('pushes a pick as a history entry that opens the archive at its root', async () => {
      const { wrapper, router } = await mountAt('/repos/7?tab=archives&path=%2Fetc')
      await wrapper.findAll('.archive-row-select')[0].trigger('click')
      await flushPromises()

      expect(selections(wrapper)).toEqual([ARCHIVE.name])
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives', browse: ARCHIVE.name })
    })

    it('follows Back to the archive picked before', async () => {
      const { wrapper, router } = await mountAt('/repos/7?tab=archives')
      await wrapper.findAll('.archive-row-select')[0].trigger('click')
      await flushPromises()
      await wrapper.setProps({ selected: ARCHIVE })
      await wrapper.findAll('.archive-row-select')[1].trigger('click')
      await flushPromises()
      await wrapper.setProps({ selected: SECOND })

      router.back()
      await flushPromises()
      expect(selections(wrapper).at(-1)).toBe(ARCHIVE.name)

      await wrapper.setProps({ selected: ARCHIVE })
      router.back()
      await flushPromises()
      expect(selections(wrapper).at(-1)).toBeNull()
    })

    it('keeps the directory Back restores in the archive picked before', async () => {
      const { wrapper, router } = await mountAt('/repos/7?tab=archives')
      await wrapper.findAll('.archive-row-select')[0].trigger('click')
      await flushPromises()
      await wrapper.setProps({ selected: ARCHIVE })
      // The file browser pushes the directory it opens.
      await router.push({ query: { ...router.currentRoute.value.query, path: '/etc' } })
      await flushPromises()
      await wrapper.findAll('.archive-row-select')[1].trigger('click')
      await flushPromises()
      await wrapper.setProps({ selected: SECOND })

      router.back()
      await flushPromises()
      await wrapper.setProps({ selected: ARCHIVE })
      await flushPromises()
      expect(router.currentRoute.value.query).toEqual({
        tab: 'archives',
        browse: ARCHIVE.name,
        path: '/etc',
      })
    })

    it('opens a linked archive once the list holding it arrives', async () => {
      const { wrapper } = await mountAt(`/repos/7?browse=${SECOND.name}&path=%2Fetc`, {
        archives: [],
      })
      expect(selections(wrapper)).toEqual([])

      await wrapper.setProps({ archives: [ARCHIVE, SECOND] })
      await flushPromises()
      expect(selections(wrapper)).toEqual([SECOND.name])
      // Handed on so the browser opens the linked directory in it.
      expect(wrapper.findComponent({ name: 'ArchiveFileBrowser' }).props('routeArchiveName')).toBe(
        SECOND.name,
      )
    })

    it('records a selection the page makes itself without a history entry', async () => {
      const { wrapper, router } = await mountAt('/repos/7?tab=archives')

      await wrapper.setProps({ selected: SECOND })
      await flushPromises()
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives', browse: SECOND.name })

      await wrapper.setProps({ selected: null })
      await flushPromises()
      expect(router.currentRoute.value.query).toEqual({ tab: 'archives' })
      // Neither change added an entry: Back leaves the page.
      router.back()
      await flushPromises()
      expect(router.currentRoute.value.fullPath).toBe('/')
    })

    it('keeps its archive under its own key when told to', async () => {
      const { wrapper, router } = await mountAt('/repos/7?browse=other', {
        pathQueryKey: 'path-7',
        archiveQueryKey: 'browse-7',
      })
      await wrapper.findAll('.archive-row-select')[0].trigger('click')
      await flushPromises()

      expect(router.currentRoute.value.query).toEqual({ browse: 'other', 'browse-7': ARCHIVE.name })
    })

    // One list can hold the same name in two repositories (a schedule's two
    // targets), so there the route names the repository too.
    describe('across repositories', () => {
      const OFFSITE_ID = 9
      const OFFSITE_COPY: RepoArchive = { ...ARCHIVE, repo_id: OFFSITE_ID, repo_name: 'offsite' }

      function repoOfLastSelection(wrapper: ReturnType<typeof mount>): number | null {
        const last = (wrapper.emitted('update:selected') ?? []).at(-1)?.[0] as RepoArchive | null
        return last?.repo_id ?? null
      }

      it('records the copy as <repo id>/<name>', async () => {
        const { wrapper, router } = await mountAt('/repos/7?tab=archives', {
          archives: [ARCHIVE, OFFSITE_COPY],
        })

        await wrapper.setProps({ selected: OFFSITE_COPY })
        await flushPromises()

        expect(router.currentRoute.value.query).toEqual({
          tab: 'archives',
          browse: `${OFFSITE_ID}/${ARCHIVE.name}`,
        })
      })

      it('opens the copy a link names, not the other repository’s', async () => {
        const { wrapper } = await mountAt(
          `/repos/7?browse=${encodeURIComponent(`${OFFSITE_ID}/${ARCHIVE.name}`)}`,
          { archives: [ARCHIVE, OFFSITE_COPY] },
        )

        expect(repoOfLastSelection(wrapper)).toBe(OFFSITE_ID)
        // The browser compares the route with the archive it shows by name.
        expect(
          wrapper.findComponent({ name: 'ArchiveFileBrowser' }).props('routeArchiveName'),
        ).toBe(ARCHIVE.name)
      })

      it('still opens a link that names the archive alone', async () => {
        const { wrapper } = await mountAt(`/repos/7?browse=${ARCHIVE.name}`, {
          archives: [ARCHIVE, OFFSITE_COPY],
        })

        expect(selections(wrapper)).toEqual([ARCHIVE.name])
      })
    })
  })
})
