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

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n} B`,
  formatDate: (s: string): string => s,
}))

vi.mock('@lucide/vue', () => ({
  Search: { template: '<span class="icon-search" />' },
}))

import { apiClient } from '../api/client'
import FileSearch from './FileSearch.vue'

const mockGet = apiClient.get as MockInstance

const ARCHIVES = [
  { name: 'web-server-01-2026-05-30T12:00:00' },
  { name: 'web-server-01-2026-05-29T12:00:00' },
]

function mountSearch(repoId: number | null = 1): ReturnType<typeof mount> {
  return mount(FileSearch, {
    props: { repoId, archives: ARCHIVES },
    global: {
      stubs: {
        DataTable: { template: '<div class="stub-datatable"><slot /></div>' },
        Column: { template: '<div class="stub-column"><slot /></div>' },
        BaseSpinner: { template: '<div class="stub-spinner" />' },
        EmptyState: {
          template: '<div class="stub-empty">{{ title }}</div>',
          props: ['icon', 'title', 'description'],
        },
      },
    },
  })
}

describe('FileSearch', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the File Search heading', () => {
    const wrapper = mountSearch()
    expect(wrapper.text()).toContain('File Search')
  })

  it('renders mode toggle buttons', () => {
    const wrapper = mountSearch()
    expect(wrapper.text()).toContain('All archives')
    expect(wrapper.text()).toContain('Single archive')
  })

  it('renders pattern input', () => {
    const wrapper = mountSearch()
    const input = wrapper.find('input[placeholder*="*.sql"]')
    expect(input.exists()).toBe(true)
  })

  it('disables Search button when pattern is empty', () => {
    const wrapper = mountSearch()
    const btn = wrapper.find('button.btn-primary')
    expect(btn.attributes('disabled')).toBeDefined()
  })

  it('enables Search button when pattern has content', async () => {
    const wrapper = mountSearch()
    const input = wrapper.find('input[placeholder*="*.sql"]')
    await input.setValue('*.conf')
    const btn = wrapper.find('button.btn-primary')
    expect(btn.attributes('disabled')).toBeUndefined()
  })

  it('calls API and displays results on Search click', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          path: '/etc/nginx/nginx.conf',
          size: 1024,
          mtime: '2026-05-30T10:00:00',
          type: '-',
          archive_name: 'web-server-01-2026-05-30T12:00:00',
        },
      ],
    })

    const wrapper = mountSearch()
    await wrapper.find('input[placeholder*="*.sql"]').setValue('*.conf')
    await wrapper.find('button.btn-primary').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(mockGet).toHaveBeenCalledWith(
      '/repos/1/search',
      expect.objectContaining({ params: expect.objectContaining({ pattern: '*.conf' }) }),
    )
    expect(wrapper.text()).toContain('1 result')
  })

  it('shows empty state when search returns no results', async () => {
    mockGet.mockResolvedValue({ data: [] })

    const wrapper = mountSearch()
    await wrapper.find('input[placeholder*="*.sql"]').setValue('*.xyz')
    await wrapper.find('button.btn-primary').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.stub-empty').exists()).toBe(true)
    expect(wrapper.find('.stub-empty').text()).toContain('No files found')
  })

  it('triggers search on Enter key', async () => {
    mockGet.mockResolvedValue({ data: [] })

    const wrapper = mountSearch()
    const input = wrapper.find('input[placeholder*="*.sql"]')
    await input.setValue('*.log')
    await input.trigger('keydown', { key: 'Enter' })
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(mockGet).toHaveBeenCalled()
  })

  it('switches to single-archive mode and shows archive selector', async () => {
    const wrapper = mountSearch()
    const singleBtn = wrapper.findAll('button.mode-btn').find((b) => b.text() === 'Single archive')
    expect(singleBtn).toBeDefined()
    await singleBtn!.trigger('click')

    const archiveSelect = wrapper.find('.archive-select-row select')
    expect(archiveSelect.exists()).toBe(true)
    const options = archiveSelect.findAll('option').map((o) => o.text())
    expect(options).toContain('web-server-01-2026-05-30T12:00:00')
  })

  it('disables Search button in single mode when no archive is selected', async () => {
    const wrapper = mountSearch()
    const singleBtn = wrapper.findAll('button.mode-btn').find((b) => b.text() === 'Single archive')
    await singleBtn!.trigger('click')
    await wrapper.find('input[placeholder*="*.sql"]').setValue('*.conf')
    const btn = wrapper.find('button.btn-primary')
    expect(btn.attributes('disabled')).toBeDefined()
  })

  it('shows error message when API fails', async () => {
    mockGet.mockRejectedValue(new Error('Network failure'))

    const wrapper = mountSearch()
    await wrapper.find('input[placeholder*="*.sql"]').setValue('*.conf')
    await wrapper.find('button.btn-primary').trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.state-error').text()).toBe('Network failure')
  })
})
