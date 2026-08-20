// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'

const mockRequestArchiveDelete = vi.fn()

vi.mock('../composables/useArchiveBrowser', () => ({
  requestArchiveDelete: (repoId: number, name: string) => mockRequestArchiveDelete(repoId, name),
}))

vi.mock('./ArchiveFileBrowser.vue', () => ({
  default: {
    name: 'ArchiveFileBrowser',
    props: ['repoId', 'archive', 'isAdmin', 'deleting'],
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

const ARCHIVE: ArchiveEntry = {
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
      repoId: 7,
      repoName: 'server-daily',
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

  it('offers deletion only to an admin working against a real repository', () => {
    expect(explorer().find('.archive-row-delete').exists()).toBe(true)
    expect(explorer({ isAdmin: false }).find('.archive-row-delete').exists()).toBe(false)
    // A schedule whose repository has not resolved yet cannot delete anything.
    expect(explorer({ repoId: null }).find('.archive-row-delete').exists()).toBe(false)
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

    wrapper.vm.onArchiveDeleted(ARCHIVE.name)
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

    wrapper.vm.onRepoIdle()
    await flushPromises()

    expect(reload).toHaveBeenCalledWith(true)
    expect(wrapper.find('.archive-row-pending').exists()).toBe(false)
  })
})
