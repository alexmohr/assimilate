// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (bytes: number): string => `${bytes} B`,
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
}))

vi.mock('../components/BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

vi.mock('../components/EmptyState.vue', () => ({
  default: {
    props: ['title', 'description'],
    template: '<div class="empty-state"><span>{{ title }}</span></div>',
  },
}))

vi.mock('../components/ToggleSwitch.vue', () => ({
  default: { template: '<input type="checkbox" />', props: ['modelValue'] },
}))

import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import ServerQuotasView from './ServerQuotasView.vue'

const mockGet = vi.mocked(apiClient.get)
const mockPut = vi.mocked(apiClient.put)
const mockDelete = vi.mocked(apiClient.delete)

const MOCK_QUOTAS = [
  {
    ssh_host: 'backup.example.com',
    warn_bytes: 10_737_418_240,
    critical_bytes: 21_474_836_480,
    warn_action: 'notify_only',
    critical_action: 'block_backups',
    enabled: true,
    updated_at: '2026-01-01T00:00:00Z',
  },
  {
    ssh_host: 'storage.local',
    warn_bytes: null,
    critical_bytes: null,
    warn_action: 'notify_only',
    critical_action: 'notify_only',
    enabled: false,
    updated_at: '2026-01-02T00:00:00Z',
  },
]

const MOCK_HOSTS = ['backup.example.com', 'storage.local', 'new-server.local']

function setupSuccessMocks(
  quotas = MOCK_QUOTAS,
  hosts = MOCK_HOSTS,
): void {
  mockGet.mockImplementation((url: string) => {
    if (url === '/server-quotas') return Promise.resolve({ data: quotas })
    if (url === '/server-quotas/hosts') return Promise.resolve({ data: hosts })
    return Promise.reject(new Error(`unexpected GET ${url}`))
  })
}

describe('ServerQuotasView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows loading state initially', async () => {
    mockGet.mockReturnValue(new Promise(() => {}))
    const wrapper = renderWithPlugins(ServerQuotasView)
    await nextTick()
    expect(wrapper.find('.base-spinner').exists()).toBe(true)
  })

  it('shows error when API fails', async () => {
    mockGet.mockRejectedValue(new Error('network error'))
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain('API error')
  })

  it('shows empty state when no quotas exist', async () => {
    mockGet.mockImplementation((url: string) => {
      if (url === '/server-quotas') return Promise.resolve({ data: [] })
      if (url === '/server-quotas/hosts') return Promise.resolve({ data: [] })
      return Promise.reject(new Error(`unexpected GET ${url}`))
    })
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.text()).toContain('No server quotas configured')
  })

  it('renders quota list with host names', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain('backup.example.com')
    expect(wrapper.text()).toContain('storage.local')
  })

  it('shows Disabled badge for disabled quotas', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain('Disabled')
  })

  it('renders action labels for quotas with byte limits', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain('Block all backups + notify')
  })

  it('Add Quota button is disabled when all hosts have quotas', async () => {
    setupSuccessMocks(MOCK_QUOTAS, ['backup.example.com', 'storage.local'])
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    expect(addBtn).toBeTruthy()
    expect((addBtn!.element as HTMLButtonElement).disabled).toBe(true)
  })

  it('Add Quota button is enabled when unconfigured hosts exist', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    expect(addBtn).toBeTruthy()
    expect((addBtn!.element as HTMLButtonElement).disabled).toBe(false)
  })

  it('opens add dialog when Add Quota is clicked', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    expect(wrapper.find('.dialog-overlay').exists()).toBe(false)

    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    await addBtn!.trigger('click')
    await nextTick()

    expect(wrapper.find('.dialog-overlay').exists()).toBe(true)
    expect(wrapper.text()).toContain('Add Server Quota')
  })

  it('add dialog lists only unconfigured hosts', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    await addBtn!.trigger('click')
    await nextTick()

    const options = wrapper
      .find('.dialog-overlay select')
      .findAll('option')
      .map((o) => o.text())
    expect(options).toContain('new-server.local')
    expect(options).not.toContain('backup.example.com')
    expect(options).not.toContain('storage.local')
  })

  it('submitting add dialog calls PUT and reloads', async () => {
    setupSuccessMocks()
    mockPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    await addBtn!.trigger('click')
    await nextTick()

    const submitBtn = wrapper.find('.dialog-overlay .btn-primary')
    await submitBtn.trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      expect.stringContaining('/server-quotas/'),
      expect.objectContaining({ enabled: true }),
    )
  })

  it('cancel add dialog hides the overlay', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    await addBtn!.trigger('click')
    await nextTick()
    expect(wrapper.find('.dialog-overlay').exists()).toBe(true)

    const cancelBtn = wrapper.find('.dialog-overlay .btn-ghost')
    await cancelBtn.trigger('click')
    await nextTick()
    expect(wrapper.find('.dialog-overlay').exists()).toBe(false)
  })

  it('clicking Edit shows inline edit form for that quota', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const editBtns = wrapper.findAll('.btn-sm').filter((b) => b.text() === 'Edit')
    expect(editBtns.length).toBeGreaterThan(0)
    await editBtns[0].trigger('click')
    await nextTick()

    expect(wrapper.find('.edit-form').exists()).toBe(true)
  })

  it('submitting edit calls PUT and reloads', async () => {
    setupSuccessMocks()
    mockPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const editBtns = wrapper.findAll('.btn-sm').filter((b) => b.text() === 'Edit')
    await editBtns[0].trigger('click')
    await nextTick()

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      expect.stringContaining('/server-quotas/'),
      expect.objectContaining({ enabled: true }),
    )
  })

  it('clicking delete icon shows confirmation dialog', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const dangerBtns = wrapper.findAll('.btn-danger')
    expect(dangerBtns.length).toBeGreaterThan(0)
    await dangerBtns[0].trigger('click')
    await nextTick()

    expect(wrapper.find('.dialog-overlay').exists()).toBe(true)
    expect(wrapper.text()).toContain('Delete Server Quota')
  })

  it('confirming delete calls DELETE endpoint and reloads', async () => {
    setupSuccessMocks()
    mockDelete.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const dangerBtns = wrapper.findAll('.btn-danger')
    await dangerBtns[0].trigger('click')
    await nextTick()

    const confirmBtn = wrapper.find('.dialog-overlay .btn-danger')
    await confirmBtn.trigger('click')
    await flushPromises()

    expect(mockDelete).toHaveBeenCalledWith(
      expect.stringContaining('/server-quotas/'),
    )
  })

  it('availableHosts excludes already-configured hosts', async () => {
    const allHosts = ['a.local', 'b.local', 'c.local']
    const existingQuotas = [
      {
        ssh_host: 'a.local',
        warn_bytes: null,
        critical_bytes: null,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: true,
        updated_at: '2026-01-01T00:00:00Z',
      },
    ]
    mockGet.mockImplementation((url: string) => {
      if (url === '/server-quotas') return Promise.resolve({ data: existingQuotas })
      if (url === '/server-quotas/hosts') return Promise.resolve({ data: allHosts })
      return Promise.reject(new Error(`unexpected GET ${url}`))
    })

    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    const addBtn = wrapper.findAll('button').find((b) => b.text().includes('Add Quota'))
    await addBtn!.trigger('click')
    await nextTick()

    const options = wrapper
      .find('.dialog-overlay select')
      .findAll('option')
      .map((o) => o.text())
    expect(options).toContain('b.local')
    expect(options).toContain('c.local')
    expect(options).not.toContain('a.local')
  })
})
