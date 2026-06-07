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
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  client_count: number
}

const mockRepos: RepoWithStats[] = [
  {
    id: 1,
    name: 'server-daily',
    repo_path: '/backup/server-daily',
    ssh_user: 'borg',
    ssh_host: 'backup.example.com',
    ssh_port: 22,
    ssh_host_key: 'ssh-ed25519 AAAA',
    compression: 'lz4',
    encryption: 'repokey-blake2',
    enabled: true,
    archive_count: 30,
    last_backup_at: new Date(Date.now() - 3_600_000).toISOString(),
    total_original_size: 10_737_418_240,
    total_compressed_size: 5_368_709_120,
    total_deduplicated_size: 2_684_354_560,
    client_count: 2,
  },
  {
    id: 2,
    name: 'database-hourly',
    repo_path: '/backup/database-hourly',
    ssh_user: 'borg',
    ssh_host: 'backup.example.com',
    ssh_port: 22,
    ssh_host_key: 'ssh-ed25519 AAAA',
    compression: 'zstd',
    encryption: 'repokey-blake2',
    enabled: true,
    archive_count: 72,
    last_backup_at: new Date(Date.now() - 300_000).toISOString(),
    total_original_size: 5_368_709_120,
    total_compressed_size: 2_684_354_560,
    total_deduplicated_size: 1_342_177_280,
    client_count: 1,
  },
  {
    id: 3,
    name: 'media-weekly',
    repo_path: '/backup/media-weekly',
    ssh_user: 'borg',
    ssh_host: 'backup.example.com',
    ssh_port: 22,
    ssh_host_key: 'ssh-ed25519 AAAA',
    compression: 'zstd',
    encryption: 'repokey-blake2',
    enabled: false,
    archive_count: 12,
    last_backup_at: null,
    total_original_size: 21_474_836_480,
    total_compressed_size: 10_737_418_240,
    total_deduplicated_size: 5_368_709_120,
    client_count: 3,
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

  it('renders archive count, size, and client stats for each repo', async () => {
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
})
