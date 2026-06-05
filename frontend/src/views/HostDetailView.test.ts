// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { ref, type ComponentPublicInstance } from 'vue'
import { renderWithPlugins } from '../test-utils'
import HostDetailView from './HostDetailView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({ onMessage: vi.fn() }),
}))

vi.mock('../composables/useEscapeKey', () => ({
  useEscapeKey: vi.fn(),
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: ref(false), copy: vi.fn() }),
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback?: string) => fallback ?? 'Unknown error',
}))

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

vi.mock('../components/MergeClientDialog.vue', () => ({
  default: {
    name: 'MergeClientDialog',
    template: '<div />',
    props: ['clients', 'current'],
  },
}))

vi.mock('../components/AgentDeployDialog.vue', () => ({
  default: {
    name: 'AgentDeployDialog',
    template: '<div />',
    props: ['hostname'],
  },
}))

import { apiClient } from '../api/client'

const mockClient = {
  id: 1,
  hostname: 'test-host',
  display_name: 'Test Host',
  agent_version: '1.0.0',
  agent_git_sha: 'abc123',
  agent_build_time: null,
  created_at: '2026-01-01T00:00:00Z',
  last_seen_at: '2026-06-03T00:00:00Z',
  is_connected: true,
  is_imported: false,
  is_hidden: false,
  supports_restart: false,
  restart_unavailable_reason: null,
  default_backup_paths: [],
  default_exclude_patterns: [],
}

const mockReports = [
  {
    id: 1,
    machine_id: 1,
    repo_id: 10,
    started_at: '2026-06-01T09:55:00Z',
    finished_at: '2026-06-01T10:00:00Z',
    status: 'success',
    original_size: 1024,
    compressed_size: 512,
    deduplicated_size: 256,
    files_processed: 100,
    duration_secs: 300,
    error_message: null,
    warnings: [],
    borg_version: '1.2.0',
    archive_name: 'test-host-2026-06-01T10:00:00',
  },
  {
    id: 2,
    machine_id: 1,
    repo_id: 10,
    started_at: '2026-06-02T09:55:00Z',
    finished_at: '2026-06-02T10:00:00Z',
    status: 'warning',
    original_size: 1024,
    compressed_size: 512,
    deduplicated_size: 256,
    files_processed: 98,
    duration_secs: 310,
    error_message: null,
    warnings: ['some file changed during backup'],
    borg_version: '1.2.0',
    archive_name: 'test-host-2026-06-02T10:00:00',
  },
  {
    id: 3,
    machine_id: 1,
    repo_id: 10,
    started_at: '2026-06-03T09:55:00Z',
    finished_at: '2026-06-03T10:00:00Z',
    status: 'failed',
    original_size: 0,
    compressed_size: 0,
    deduplicated_size: 0,
    files_processed: 0,
    duration_secs: 5,
    error_message: 'Connection refused',
    warnings: [],
    borg_version: '1.2.0',
    archive_name: null,
  },
]

function setupApi(reports = mockReports): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/clients') return Promise.resolve({ data: [mockClient] })
    if (url === '/clients/test-host/repos') return Promise.resolve({ data: [] })
    if (url === '/clients/test-host/schedules') return Promise.resolve({ data: [] })
    if (url === '/clients/test-host/reports') return Promise.resolve({ data: reports })
    if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
    if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
    return Promise.resolve({ data: [] })
  })
}

async function openBackupsTab(wrapper: VueWrapper<ComponentPublicInstance>): Promise<void> {
  const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
  await router.push({ query: { tab: 'backups' } })
  await flushPromises()
}

describe('HostDetailView — backups tab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows filter buttons on the backups tab', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const text = wrapper.text()
    expect(text).toContain('All')
    expect(text).toContain('Success')
    expect(text).toContain('Warning')
    expect(text).toContain('Failed')
  })

  it('shows sort toggle button on the backups tab', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    expect(wrapper.text()).toMatch(/Newest|Oldest/)
  })

  it('renders all reports by default', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    expect(wrapper.findAll('.result-card')).toHaveLength(3)
  })

  it('filters to only warning reports when Warning is clicked', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const warningBtn = wrapper.findAll('button').find((b) => b.text() === 'Warning')
    await warningBtn!.trigger('click')

    const cards = wrapper.findAll('.result-card')
    expect(cards).toHaveLength(1)
    expect(cards[0].classes()).toContain('result-warning')
  })

  it('filters to only failed reports when Failed is clicked', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const failedBtn = wrapper.findAll('button').find((b) => b.text() === 'Failed')
    await failedBtn!.trigger('click')

    const cards = wrapper.findAll('.result-card')
    expect(cards).toHaveLength(1)
    expect(cards[0].classes()).toContain('result-failed')
  })

  it('restores all reports when All is clicked after filtering', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Warning')!
      .trigger('click')
    expect(wrapper.findAll('.result-card')).toHaveLength(1)

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'All')!
      .trigger('click')
    expect(wrapper.findAll('.result-card')).toHaveLength(3)
  })

  it('shows empty filter message when no reports match the filter', async () => {
    setupApi([mockReports[0]])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Failed')!
      .trigger('click')

    expect(wrapper.text()).toContain('No backups match the current filter.')
  })

  it('shows empty state when no reports exist', async () => {
    setupApi([])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    expect(wrapper.text()).toContain('No backup reports available.')
  })

  it('highlights the report matching the archive query param', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    // Navigate to backups tab with archive query param via the router
    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups', archive: 'test-host-2026-06-02T10:00:00' } })
    await flushPromises()

    const highlighted = wrapper.find('.result-card-highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.classes()).toContain('result-warning')
  })

  it('auto-expands the report matching the archive query param', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups', archive: 'test-host-2026-06-02T10:00:00' } })
    await flushPromises()

    expect(wrapper.text()).toContain('some file changed during backup')
  })
})
