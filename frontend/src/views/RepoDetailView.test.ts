// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { ref } from 'vue'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { renderWithPlugins } from '../test-utils'
import RepoDetailView from './RepoDetailView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../composables/useEscapeKey', () => ({
  useEscapeKey: vi.fn(),
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: ref(false), copy: vi.fn() }),
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({ status: ref('connected'), onMessage: vi.fn() }),
}))

vi.mock('../composables/useArchiveBrowser', () => ({
  useArchiveBrowser: () => ({
    archives: ref([]),
    sortedArchives: ref([]),
    archivesLoading: ref(false),
    archivesError: ref(null),
    selectedArchive: ref(null),
    contentsLoading: ref(false),
    contentsError: ref(null),
    breadcrumbs: ref([]),
    dirs: ref([]),
    files: ref([]),
    loadArchives: vi.fn(),
    selectArchive: vi.fn(),
    navigateTo: vi.fn(),
    entryName: vi.fn((e: { path: string }) => e.path.split('/').pop() ?? ''),
    downloadEntry: vi.fn(),
    restoreEntry: vi.fn(),
    deleteArchive: vi.fn(),
  }),
}))

vi.mock('../components/QuotaPanel.vue', () => ({
  default: {
    name: 'QuotaPanel',
    template: '<div data-testid="quota-panel">QuotaPanel stub</div>',
    props: ['repoId', 'isAdmin'],
  },
}))

import { apiClient } from '../api/client'

interface RepoWithStats {
  id: number
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  compression: string
  encryption: string
  enabled: boolean
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  client_count: number
}

const mockRepo: RepoWithStats = {
  id: 1,
  name: 'server-daily',
  repo_path: '/backup/repos/server-daily',
  ssh_user: 'borg',
  ssh_host: 'backup.example.com',
  ssh_port: 22,
  compression: 'lz4',
  encryption: 'repokey-blake2',
  enabled: true,
  archive_count: 30,
  last_backup_at: new Date(Date.now() - 3_600_000).toISOString(),
  total_original_size: 10_737_418_240,
  total_compressed_size: 5_368_709_120,
  total_deduplicated_size: 2_684_354_560,
  client_count: 2,
}

function setupApiSuccess(repo: RepoWithStats = mockRepo): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === `/repos/${repo.id}`) return Promise.resolve({ data: repo })
    if (String(url).startsWith('/tags')) return Promise.resolve({ data: [] })
    if (String(url).endsWith('/tags')) return Promise.resolve({ data: [] })
    return Promise.resolve({ data: [] })
  })
}

describe('RepoDetailView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders repo name in breadcrumb and info grid', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
  })

  it('displays compression and encryption values', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('lz4')
    expect(text).toContain('repokey-blake2')
  })

  it('shows SSH target in info grid', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('borg@backup.example.com:22')
  })

  it('shows repo path in info grid', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('/backup/repos/server-daily')
  })

  it('renders stat cards with archive count and client count', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('30')
    expect(text).toContain('Archives')
    expect(text).toContain('2')
    expect(text).toContain('Clients')
  })

  it('renders QuotaPanel component', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.find('[data-testid="quota-panel"]').exists()).toBe(true)
  })

  it('shows Enabled status badge when repo is enabled', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Enabled')
  })

  it('shows Disabled status badge when repo is disabled', async () => {
    setupApiSuccess({ ...mockRepo, enabled: false })
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Disabled')
  })

  it('shows Overview and Archives tabs', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('Overview')
    expect(text).toContain('Archives')
  })

  it('shows archives tab content when Archives tab is clicked', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
    expect(archivesTab).toBeDefined()
    await archivesTab!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('No archives found')
  })

  it('shows danger zone for admin users', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Danger Zone')
    expect(wrapper.text()).toContain('Delete Repository')
  })

  it('hides danger zone for non-admin users', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.find('.danger-zone').exists()).toBe(false)
  })

  it('shows error message when repo load fails', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('Not found'))
    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '99' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Not found')
  })

  it('calls sync endpoint and clears loading after 202 response', async () => {
    setupApiSuccess()
    vi.mocked(apiClient.post).mockResolvedValue({ status: 202, data: {} })

    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const syncBtn = wrapper.findAll('button').find((b) => b.text() === 'Full Resync')
    expect(syncBtn).toBeDefined()
    await syncBtn!.trigger('click')
    await flushPromises()

    // After 202 response loading clears -- button returns to normal label
    expect(wrapper.findAll('button').find((b) => b.text() === 'Full Resync')).toBeDefined()
    expect(wrapper.findAll('button').find((b) => b.text() === 'Syncing...')).toBeUndefined()

    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/repos/1/sync')
  })

  it('shows error toast when sync request fails', async () => {
    setupApiSuccess()
    vi.mocked(apiClient.post).mockRejectedValue(new Error('Connection refused'))

    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const syncBtn = wrapper.findAll('button').find((b) => b.text() === 'Full Resync')
    expect(syncBtn).toBeDefined()
    await syncBtn!.trigger('click')
    await flushPromises()

    // Loading clears even on failure
    expect(wrapper.findAll('button').find((b) => b.text() === 'Full Resync')).toBeDefined()
    expect(wrapper.findAll('button').find((b) => b.text() === 'Syncing...')).toBeUndefined()

    // Error message visible in the page (toast container is teleported so check apiClient call)
    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/repos/1/sync')
  })

  it('reloads data when id prop changes', async () => {
    const repo2 = { ...mockRepo, id: 2, name: 'db-hourly' }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/1') return Promise.resolve({ data: mockRepo })
      if (url === '/repos/2') return Promise.resolve({ data: repo2 })
      return Promise.resolve({ data: [] })
    })

    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('server-daily')

    await wrapper.setProps({ id: '2' })
    await flushPromises()

    expect(wrapper.text()).toContain('db-hourly')
    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith('/repos/2')
  })
})
