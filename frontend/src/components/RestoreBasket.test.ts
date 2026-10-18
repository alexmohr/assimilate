// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest'
import { flushPromises, mount, RouterLinkStub } from '@vue/test-utils'
import { mockApiClientRead, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'
import { pushWs, resetWsMock } from '../test-utils/wsMock'

vi.mock('../api/client', () => mockApiClientRead())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

import { apiClient } from '../api/client'
import RestoreBasket from './RestoreBasket.vue'
import { makeRestoreRun } from '../test-utils/restoreRun'
import type {
  ArchiveEntryResponse as ArchiveEntry,
  ContentEntryResponse as ContentEntry,
} from '../types/generated'

const mockPost = apiClient.post as MockInstance
const mockGet = apiClient.get as MockInstance

const ARCHIVE: ArchiveEntry = {
  name: 'web-01-2026-05-30',
  start: '2026-05-30T02:00:00',
  hostname: 'web-01',
  comment: '',
  original_size: 1,
  deduplicated_size: 1,
  matched: true,
  agent_hostname: 'web-01',
  agent_domain: 'dc1.example.com',
}

const ENTRIES: Record<string, ContentEntry> = {
  'etc/hosts': { path: 'etc/hosts', type: '-', size: 100, mtime: '', mode: '' },
  'var/www': { path: 'var/www', type: 'd', size: 0, mtime: '', mode: '' },
}

function mountBasket(overrides: Record<string, unknown> = {}) {
  return mount(RestoreBasket, {
    props: {
      repoId: 5,
      archive: ARCHIVE,
      paths: ['etc/hosts', 'var/www'],
      entryOf: (path: string) => ENTRIES[path] ?? null,
      totals: { 'var/www': { path: 'var/www', files: 3, bytes: 2048 } },
      isAdmin: true,
      ...overrides,
    },
    attachTo: document.body,
    global: {
      stubs: {
        RouterLink: RouterLinkStub,
        BaseModal: {
          template:
            '<div v-if="open" class="stub-modal"><button type="button" aria-label="Close" @click="$emit(\'close\')" /><slot /><slot name="footer" /></div>',
          props: ['open', 'title', 'size'],
          emits: ['close'],
        },
      },
    },
  })
}

function restoreButton(wrapper: ReturnType<typeof mountBasket>) {
  const button = wrapper.findAll('button').find((b) => b.text().includes('Restore to host'))
  if (button === undefined) throw new Error('no Restore to host button')
  return button
}

function modalButton(wrapper: ReturnType<typeof mountBasket>, label: string) {
  const button = wrapper
    .get('.stub-modal')
    .findAll('button')
    .find((b) => b.text() === label)
  if (button === undefined) throw new Error(`no ${label} button in the confirmation`)
  return button
}

describe('RestoreBasket', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetWsMock()
  })

  it('lists each entry with its size and adds up the basket', () => {
    const wrapper = mountBasket()

    const items = wrapper.findAll('.basket-item')
    expect(items.map((item) => item.attributes('data-path'))).toEqual(['etc/hosts', 'var/www'])
    expect(items[1]!.text()).toContain('3 files')
    expect(wrapper.get('[data-testid="basket-total"]').text()).toContain('4 files, 2.1 KB')
    wrapper.unmount()
  })

  it('says it is still counting while a folder total is unknown', () => {
    const wrapper = mountBasket({ totals: {} })

    expect(wrapper.get('[data-testid="basket-total"]').text()).toContain('Counting...')
    wrapper.unmount()
  })

  it('removes an entry', async () => {
    const wrapper = mountBasket()

    await wrapper.get('[aria-label="Remove etc/hosts"]').trigger('click')

    expect(wrapper.emitted('remove')).toEqual([['etc/hosts']])
    wrapper.unmount()
  })

  it('downloads the whole basket from one streamed link', () => {
    const wrapper = mountBasket()

    const href = wrapper.get('[data-testid="basket-download"]').attributes('href') ?? ''
    const url = new URL(href, 'http://localhost')
    expect(url.pathname).toBe('/api/repos/5/archives/web-01-2026-05-30/download')
    expect(JSON.parse(url.searchParams.get('paths') ?? '[]')).toEqual(['etc/hosts', 'var/www'])
    wrapper.unmount()
  })

  it('refuses a relative destination before anything is sent', async () => {
    const wrapper = mountBasket()

    await wrapper.get('#restore-destination').setValue('srv/restore')

    expect(wrapper.find('.form-error').text()).toContain('absolute path')
    expect(restoreButton(wrapper).attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-testid="basket-download"]').attributes('href')).toBeUndefined()
    wrapper.unmount()
  })

  it('confirms the destination, then restores every path in one request and follows it', async () => {
    const started = makeRestoreRun({
      hostname: 'web-01',
      paths: ['etc/hosts', 'var/www'],
      target_path: '/srv/restore',
      status: 'running',
    })
    mockPost.mockResolvedValue({ data: started })
    mockGet.mockResolvedValue({ data: started })
    const wrapper = mountBasket()

    await wrapper.get('#restore-destination').setValue('/srv/restore/')
    await restoreButton(wrapper).trigger('click')
    const modal = wrapper.get('.stub-modal')
    expect(modal.get('[data-testid="restore-confirm-destination"]').text()).toBe('/srv/restore')
    expect(modal.text()).toContain('/srv/restore/etc/hosts')
    expect(modal.text()).toContain('Existing files under /srv/restore are overwritten')

    await modalButton(wrapper, 'Restore').trigger('click')
    await flushPromises()

    expect(mockPost).toHaveBeenCalledWith('/repos/5/archives/web-01-2026-05-30/restore', {
      paths: ['etc/hosts', 'var/www'],
      target_path: '/srv/restore',
      hostname: 'web-01',
      domain: 'dc1.example.com',
    })
    mockGet.mockResolvedValue({ data: { ...started, status: 'success' } })
    pushWs('RestoreRunChanged', { id: started.id, status: 'success' })
    await flushPromises()
    expect(wrapper.get('[data-testid="basket-restore-status"]').text()).toContain('Restored')
    wrapper.unmount()
  })

  it('warns about overwriting the original locations when there is no destination', async () => {
    const wrapper = mountBasket()

    await restoreButton(wrapper).trigger('click')

    const modal = wrapper.get('.stub-modal')
    expect(modal.get('[data-testid="restore-confirm-destination"]').text()).toBe(
      'Original locations',
    )
    expect(modal.text()).toContain('/etc/hosts')
    expect(modal.text()).toContain('original locations are overwritten')
    wrapper.unmount()
  })

  it('offers only the download to non-admins and for archives no agent claims', () => {
    const viewer = mountBasket({ isAdmin: false })
    expect(viewer.text()).not.toContain('Restore to host')
    viewer.unmount()

    const unmatched = mountBasket({ archive: { ...ARCHIVE, matched: false } })
    expect(restoreButton(unmatched).attributes('disabled')).toBeDefined()
    unmatched.unmount()
  })

  it('empties the basket on Clear', async () => {
    const wrapper = mountBasket()

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Clear')!
      .trigger('click')

    expect(wrapper.emitted('clear')).toEqual([[]])
    wrapper.unmount()
  })

  it('explains where a path lands under the destination', async () => {
    const wrapper = mountBasket()

    await wrapper.get('[aria-label="Help: About the destination"]').trigger('click')

    expect(wrapper.get('[role="note"]').text()).toContain('/srv/restore/etc/nginx/nginx.conf')
    wrapper.unmount()
  })

  it.each([
    [
      'Cancel',
      (wrapper: ReturnType<typeof mountBasket>) => modalButton(wrapper, 'Cancel').trigger('click'),
    ],
    [
      'the modal closing',
      (wrapper: ReturnType<typeof mountBasket>) =>
        wrapper.get('.stub-modal [aria-label="Close"]').trigger('click'),
    ],
  ])('closes the confirmation on %s without restoring', async (_, close) => {
    const wrapper = mountBasket()
    await restoreButton(wrapper).trigger('click')

    await close(wrapper)
    await flushPromises()

    expect(wrapper.find('.stub-modal').exists()).toBe(false)
    expect(mockPost).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it("drops the last archive's restore status and error when another archive is opened", async () => {
    const started = makeRestoreRun({ hostname: 'web-01', paths: ['etc/hosts'], status: 'running' })
    mockPost.mockResolvedValueOnce({ data: started })
    mockGet.mockResolvedValue({ data: started })
    const wrapper = mountBasket()
    await restoreButton(wrapper).trigger('click')
    await modalButton(wrapper, 'Restore').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="basket-restore-status"]').exists()).toBe(true)
    mockPost.mockRejectedValueOnce(new Error('agent offline'))
    await restoreButton(wrapper).trigger('click')
    await modalButton(wrapper, 'Restore').trigger('click')
    await flushPromises()
    expect(wrapper.get('.stub-modal .form-error').text()).toBe('agent offline')

    await wrapper.setProps({ archive: { ...ARCHIVE, name: 'web-01-2026-05-31' } })

    expect(wrapper.find('[data-testid="basket-restore-status"]').exists()).toBe(false)
    expect(wrapper.find('.form-error').exists()).toBe(false)
    wrapper.unmount()
  })
})
