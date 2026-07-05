// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, type MockInstance } from 'vitest'
import { mount } from '@vue/test-utils'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (e: unknown): string => {
    if (e instanceof Error) return e.message
    return 'Unknown error'
  },
  extractBlobError: async (e: unknown): Promise<string> => {
    if (e instanceof Error) return e.message
    return 'Unknown error'
  },
}))

import { apiClient } from '../api/client'
import ArchiveDiff from './ArchiveDiff.vue'

const mockGet = apiClient.get as MockInstance

const ARCHIVES = [
  {
    name: 'web-server-01-2026-05-30T12:00:00',
    start: '2026-05-30T12:00:00',
    hostname: 'web-server-01',
    comment: 'pre-upgrade',
  },
  {
    name: 'web-server-01-2026-05-29T12:00:00',
    start: '2026-05-29T12:00:00',
    hostname: 'web-server-01',
    comment: 'weekly-baseline',
  },
]

function mountDiff(open = true): ReturnType<typeof mount> {
  return mount(ArchiveDiff, {
    props: { open, repoId: 1, archives: ARCHIVES },
    attachTo: document.body,
    global: {
      stubs: {
        BaseModal: {
          template: '<div v-if="open" class="stub-modal"><slot /><slot name="footer" /></div>',
          props: ['open', 'title', 'size'],
        },
        Teleport: true,
      },
    },
  })
}

describe('ArchiveDiff', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders two archive selects when open', () => {
    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    expect(selects).toHaveLength(2)
  })

  it('renders all archive options in both selects', () => {
    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    for (const select of selects) {
      const names = select
        .findAll('option')
        .map((o) => o.text())
        .filter((t) => t.includes('web-server'))
      expect(names).toHaveLength(2)
    }
  })

  it('disables Compare button when no archives are selected', () => {
    const wrapper = mountDiff()
    const btn = wrapper.find('button.compare-btn')
    expect(btn.attributes('disabled')).toBeDefined()
  })

  it('disables Compare button when the same archive is selected for both', async () => {
    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    await selects[0].setValue(ARCHIVES[0].name)
    await selects[1].setValue(ARCHIVES[0].name)
    const btn = wrapper.find('button.compare-btn')
    expect(btn.attributes('disabled')).toBeDefined()
  })

  it('enables Compare button when two different archives are selected', async () => {
    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    await selects[0].setValue(ARCHIVES[0].name)
    await selects[1].setValue(ARCHIVES[1].name)
    const btn = wrapper.find('button.compare-btn')
    expect(btn.attributes('disabled')).toBeUndefined()
  })

  it('displays added, removed and modified items after a successful diff', async () => {
    mockGet.mockResolvedValue({
      data: {
        added: ['/etc/nginx/new.conf'],
        removed: ['/etc/nginx/old.conf'],
        modified: ['/etc/nginx/nginx.conf'],
      },
    })

    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    await selects[0].setValue(ARCHIVES[0].name)
    await selects[1].setValue(ARCHIVES[1].name)
    await wrapper.find('button.compare-btn').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('/etc/nginx/new.conf')
    expect(wrapper.text()).toContain('/etc/nginx/old.conf')
    expect(wrapper.text()).toContain('/etc/nginx/nginx.conf')
    expect(wrapper.text()).toContain('Added (1)')
    expect(wrapper.text()).toContain('Removed (1)')
    expect(wrapper.text()).toContain('Modified (1)')
  })

  it('shows "No differences found" when diff result is empty', async () => {
    mockGet.mockResolvedValue({
      data: { added: [], removed: [], modified: [] },
    })

    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    await selects[0].setValue(ARCHIVES[0].name)
    await selects[1].setValue(ARCHIVES[1].name)
    await wrapper.find('button.compare-btn').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('No differences found between the two archives.')
  })

  it('shows error message when API call fails', async () => {
    mockGet.mockRejectedValue(new Error('Server error'))

    const wrapper = mountDiff()
    const selects = wrapper.findAll('select')
    await selects[0].setValue(ARCHIVES[0].name)
    await selects[1].setValue(ARCHIVES[1].name)
    await wrapper.find('button.compare-btn').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.form-error').text()).toBe('Server error')
  })

  it('does not render content when open is false', () => {
    const wrapper = mountDiff(false)
    expect(wrapper.find('.stub-modal').exists()).toBe(false)
  })
})
