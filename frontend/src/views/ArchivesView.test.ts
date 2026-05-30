// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, type MockInstance } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createRouter, createMemoryHistory } from 'vue-router'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
  },
}))

vi.mock('../composables/useEscapeKey', () => ({
  useEscapeKey: vi.fn(),
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({
    copied: { value: false },
    copy: vi.fn(),
  }),
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n} B`,
  formatDate: (s: string): string => s,
}))

vi.mock('@primevue/core/api', () => ({
  FilterMatchMode: { CONTAINS: 'contains' },
}))

import { apiClient } from '../api/client'
import ArchivesView from './ArchivesView.vue'

const mockGet = apiClient.get as MockInstance

const REPOS = [
  { id: 1, hostname: 'web-server-01', target_name: 'server-daily', enabled: true },
  { id: 2, hostname: 'db-server-01', target_name: 'database-hourly', enabled: true },
]

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

function createTestRouter(): ReturnType<typeof createRouter> {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/:pathMatch(.*)*', component: { template: '<div />' } }],
  })
}

function mountView(): ReturnType<typeof mount> {
  return mount(ArchivesView, {
    global: {
      plugins: [createPinia(), createTestRouter()],
      stubs: {
        DataTable: { template: '<div class="stub-datatable"><slot /></div>' },
        Column: { template: '<div class="stub-column"><slot /></div>' },
        BaseSpinner: { template: '<div class="stub-spinner" />' },
        RestoreWizard: { template: '<div class="stub-restore" />' },
        ArchiveDiff: { template: '<div class="stub-diff" />' },
        FileSearch: { template: '<div class="stub-search" />' },
        Teleport: true,
      },
    },
  })
}

async function pickFirstRepo(wrapper: ReturnType<typeof mount>): Promise<void> {
  const select = wrapper.find('select')
  const repoOption = wrapper
    .findAll('option')
    .find((o) => (o.element as HTMLOptionElement & { _value?: number })._value === 1)
  if (repoOption) {
    ;(repoOption.element as HTMLOptionElement).selected = true
  }
  await select.trigger('change')
  await flushPromises()
}

describe('ArchivesView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockGet.mockResolvedValue({ data: [] })
  })

  it('renders the page title', async () => {
    const wrapper = mountView()
    expect(wrapper.find('h1').text()).toBe('Archives')
  })

  it('calls loadRepos on mount', async () => {
    mountView()
    expect(mockGet).toHaveBeenCalledWith('/repos')
  })

  it('shows loading state while fetching repos', async () => {
    mockGet.mockReturnValue(new Promise(() => undefined))
    const wrapper = mountView()
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Loading repositories...')
  })

  it('renders repository options after loading', async () => {
    mockGet.mockResolvedValue({ data: REPOS })
    const wrapper = mountView()
    await flushPromises()

    const options = wrapper.findAll('option').filter((o) => o.text().includes('web-server-01'))
    expect(options.length).toBeGreaterThan(0)
    expect(wrapper.text()).toContain('web-server-01 / server-daily')
  })

  it('shows "No repositories configured yet." hint when repos list is empty', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = mountView()
    await flushPromises()

    expect(wrapper.text()).toContain('No repositories configured yet.')
  })

  it('shows archive list with archive names including demo tags after repo selection', async () => {
    mockGet.mockImplementation((url: string) => {
      if (url === '/repos') return Promise.resolve({ data: REPOS })
      if (url.includes('/archives')) return Promise.resolve({ data: ARCHIVES })
      return Promise.resolve({ data: [] })
    })

    const wrapper = mountView()
    await flushPromises()
    await pickFirstRepo(wrapper)

    expect(wrapper.text()).toContain('web-server-01-2026-05-30T12:00:00')
    expect(wrapper.text()).toContain('web-server-01-2026-05-29T12:00:00')
  })

  it('shows "No archives found." empty state when archives list is empty', async () => {
    mockGet.mockImplementation((url: string) => {
      if (url === '/repos') return Promise.resolve({ data: REPOS })
      return Promise.resolve({ data: [] })
    })

    const wrapper = mountView()
    await flushPromises()
    await pickFirstRepo(wrapper)

    expect(wrapper.text()).toContain('No archives found.')
  })

  it('sorts archives newest-first', async () => {
    mockGet.mockImplementation((url: string) => {
      if (url === '/repos') return Promise.resolve({ data: REPOS })
      if (url.includes('/archives')) return Promise.resolve({ data: ARCHIVES })
      return Promise.resolve({ data: [] })
    })

    const wrapper = mountView()
    await flushPromises()
    await pickFirstRepo(wrapper)

    const rows = wrapper.findAll('tbody tr')
    expect(rows[0].text()).toContain('2026-05-30')
    expect(rows[1].text()).toContain('2026-05-29')
  })

  it('shows an error message when repo loading fails', async () => {
    mockGet.mockRejectedValue({ response: { data: { error: 'Connection refused' } } })
    const wrapper = mountView()
    await flushPromises()

    expect(wrapper.find('.state-error').exists()).toBe(true)
  })
})
