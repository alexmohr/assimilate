// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { renderWithPlugins } from '../test-utils'
import ReposView from './ReposView.vue'

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

vi.mock('../composables/useMobile', () => ({
  useMobile: () => ({ isMobile: { value: false } }),
}))

import { apiClient } from '../api/client'

interface RepoWithStats {
  id: number
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  ssh_host_key: string | null
  compression: string
  encryption: string
  enabled: boolean
  importing: boolean
  import_error: string | null
  import_progress: number
  import_total: number
  import_status_message: string | null
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  agent_count: number
  unmatched_count: number
}

const baseRepo = {
  ssh_user: 'borg',
  ssh_host: 'backup.example.com',
  ssh_port: 22,
  ssh_host_key: 'ssh-ed25519 AAAA',
  compression: 'lz4',
  encryption: 'repokey-blake2',
  enabled: true,
  importing: false,
  import_error: null,
  import_progress: 0,
  import_total: 0,
  import_status_message: null,
  last_backup_at: null,
  total_original_size: 10_737_418_240,
  total_compressed_size: 5_368_709_120,
  total_deduplicated_size: 2_684_354_560,
  agent_count: 1,
  unmatched_count: 0,
}

const mockRepos: RepoWithStats[] = [
  {
    ...baseRepo,
    id: 1,
    name: 'server-daily',
    repo_path: '/backup/server-daily',
    compression: 'lz4',
    archive_count: 30,
    last_backup_at: new Date(Date.now() - 3_600_000).toISOString(),
    agent_count: 2,
  },
  {
    ...baseRepo,
    id: 2,
    name: 'database-hourly',
    repo_path: '/backup/database-hourly',
    compression: 'zstd',
    archive_count: 72,
    last_backup_at: new Date(Date.now() - 300_000).toISOString(),
  },
  {
    ...baseRepo,
    id: 3,
    name: 'media-weekly',
    repo_path: '/backup/media-weekly',
    compression: 'zstd',
    enabled: false,
    archive_count: 12,
    agent_count: 3,
    total_original_size: 21_474_836_480,
    total_compressed_size: 10_737_418_240,
    total_deduplicated_size: 5_368_709_120,
  },
]

function setupApiSuccess(repos: RepoWithStats[] = mockRepos): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/repos/stats') return Promise.resolve({ data: repos })
    if (url === '/repo-tags') return Promise.resolve({ data: [] })
    if (String(url).startsWith('/tags')) return Promise.resolve({ data: [] })
    return Promise.resolve({ data: [] })
  })
}

describe('ReposView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders repository list after loading', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).toContain('media-weekly')
  })

  it('displays compression and encryption metadata pills', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('lz4')
    expect(text).toContain('zstd')
    expect(text).toContain('repokey-blake2')
  })

  it('shows enabled and disabled status badges', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('Enabled')
    expect(text).toContain('Disabled')
  })

  it('renders archive count, size, and agent stats for each repo', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('30')
    expect(text).toContain('72')
    expect(text).toContain('12')
  })

  it('shows empty state when no repositories exist', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/stats') return Promise.resolve({ data: [] })
      if (url === '/repo-tags') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('No repositories configured')
  })

  it('shows "no match" message when filter text has no matches', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const input = wrapper.find('input.search-input')
    await input.setValue('zzz-does-not-exist')
    await flushPromises()

    expect(wrapper.text()).toContain('No repositories match the current filter')
  })

  it('shows New and Import buttons for admin users', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('New')
    expect(text).toContain('Import')
  })

  it('hides New and Import buttons for non-admin users', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    const headerActions = wrapper.find('.header-actions')
    expect(headerActions.exists()).toBe(false)
  })

  it('filters repos by name using search input', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    const input = wrapper.find('input.search-input')
    await input.setValue('media')
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('media-weekly')
    expect(text).not.toContain('server-daily')
    expect(text).not.toContain('database-hourly')
  })

  it('shows last backup time for repos with a backup', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toMatch(/\d+[mh] ago|Just now/)
  })

  it('shows "Never" for repos with no backup', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Never')
  })

  it('shows "Importing\u2026" badge when repo is importing without progress', async () => {
    const importingRepo: RepoWithStats = {
      ...baseRepo,
      id: 4,
      name: 'importing-repo',
      repo_path: '/backup/importing-repo',
      archive_count: 0,
      importing: true,
      import_total: 0,
    }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/stats') return Promise.resolve({ data: [importingRepo] })
      if (url === '/repo-tags') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Importing\u2026')
  })

  it('shows "Importing N/M" badge when repo is importing with progress', async () => {
    const importingRepo: RepoWithStats = {
      ...baseRepo,
      id: 4,
      name: 'importing-repo',
      repo_path: '/backup/importing-repo',
      archive_count: 0,
      importing: true,
      import_progress: 42,
      import_total: 100,
    }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/stats') return Promise.resolve({ data: [importingRepo] })
      if (url === '/repo-tags') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Importing 42/100')
  })

  it('shows "Indexing\u2026" badge when repo is in the indexing phase without progress', async () => {
    const indexingRepo: RepoWithStats = {
      ...baseRepo,
      id: 4,
      name: 'indexing-repo',
      repo_path: '/backup/indexing-repo',
      archive_count: 0,
      importing: true,
      import_total: 0,
      import_status_message: 'Indexing archive contents\u2026',
    }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/stats') return Promise.resolve({ data: [indexingRepo] })
      if (url === '/repo-tags') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Indexing\u2026')
    expect(wrapper.text()).not.toContain('Importing')
  })

  it('shows "Indexing N/M" badge when repo is in the indexing phase with progress', async () => {
    const indexingRepo: RepoWithStats = {
      ...baseRepo,
      id: 4,
      name: 'indexing-repo',
      repo_path: '/backup/indexing-repo',
      archive_count: 0,
      importing: true,
      import_progress: 10,
      import_total: 50,
      import_status_message: 'Indexing archive contents\u2026',
    }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/stats') return Promise.resolve({ data: [indexingRepo] })
      if (url === '/repo-tags') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Indexing 10/50')
    expect(wrapper.text()).not.toContain('Importing')
  })
})
