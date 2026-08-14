// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { ref } from 'vue'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { renderWithPlugins } from '../test-utils'
import ReposView from './ReposView.vue'

vi.mock('../api/client')

vi.mock('../composables/useEscapeKey')

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn> } => ({
    onMessage: vi.fn(),
  }),
}))

// A real ref, not a plain `{ value: false }` object - the template relies on Vue's
// auto-unwrapping of genuine refs (e.g. `v-if="isMobile"`), which a plain object bypasses.
vi.mock('../composables/useMobile', () => ({
  useMobile: () => ({ isMobile: ref(false) }),
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
  quota: {
    warn_bytes: number | null
    critical_bytes: number | null
    warn_action: string
    critical_action: string
    enabled: boolean
  } | null
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
  quota: null,
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

  it('shows nothing for an enabled repo and a Disabled pill for a disabled one', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const cards = wrapper.findAll('.repo-card')
    const enabledCard = cards.find((c) => c.text().includes('server-daily'))
    const disabledCard = cards.find((c) => c.text().includes('media-weekly'))

    expect(enabledCard!.find('.entity-status-pill').exists()).toBe(false)
    expect(enabledCard!.classes()).not.toContain('repo-card-notable')

    expect(disabledCard!.find('.entity-status-pill').text()).toBe('Disabled')
    expect(disabledCard!.classes()).toContain('repo-card-notable')
  })

  it('shows a clickable unmatched-archives chip that navigates to the Archives tab', async () => {
    setupApiSuccess([
      {
        ...mockRepos[0],
        id: 4,
        name: 'unmatched-repo',
        repo_path: '/backup/unmatched',
        unmatched_count: 2,
      },
    ])
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const chip = wrapper.find('.entity-issue-chip.sev-warning')
    expect(chip.text()).toBe('2 unmatched')

    await chip.trigger('click')
    await flushPromises()

    const router = (
      wrapper.vm as unknown as {
        $router: { currentRoute: { value: { path: string; query: Record<string, string> } } }
      }
    ).$router
    expect(router.currentRoute.value.path).toBe('/repos/4')
    expect(router.currentRoute.value.query).toMatchObject({ tab: 'archives' })
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
    setupApiSuccess([])
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

  it('shows "Importing…" badge when repo is importing without progress', async () => {
    const importingRepo: RepoWithStats = {
      ...baseRepo,
      id: 4,
      name: 'importing-repo',
      repo_path: '/backup/importing-repo',
      archive_count: 0,
      importing: true,
      import_total: 0,
    }
    setupApiSuccess([importingRepo])
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Importing…')
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
    setupApiSuccess([importingRepo])
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Importing 42/100')
  })

  it('shows "Indexing…" badge when repo is in the indexing phase without progress', async () => {
    const indexingRepo: RepoWithStats = {
      ...baseRepo,
      id: 4,
      name: 'indexing-repo',
      repo_path: '/backup/indexing-repo',
      archive_count: 0,
      importing: true,
      import_total: 0,
      import_status_message: 'Indexing archive contents…',
    }
    setupApiSuccess([indexingRepo])
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Indexing…')
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
      import_status_message: 'Indexing archive contents…',
    }
    setupApiSuccess([indexingRepo])
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Indexing 10/50')
    expect(wrapper.text()).not.toContain('Importing')
  })
})

describe('ReposView quota filter chips', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  const reposWithQuota: RepoWithStats[] = [
    {
      ...baseRepo,
      id: 1,
      name: 'over-warn',
      repo_path: '/backup/over-warn',
      total_deduplicated_size: 600,
      quota: {
        warn_bytes: 500,
        critical_bytes: 1000,
        warn_action: 'notify_only',
        critical_action: 'block_backups',
        enabled: true,
      },
    },
    {
      ...baseRepo,
      id: 2,
      name: 'healthy',
      repo_path: '/backup/healthy',
      total_deduplicated_size: 100,
      quota: {
        warn_bytes: 500,
        critical_bytes: 1000,
        warn_action: 'notify_only',
        critical_action: 'block_backups',
        enabled: true,
      },
    },
    {
      ...baseRepo,
      id: 3,
      name: 'unconfigured',
      repo_path: '/backup/unconfigured',
      total_deduplicated_size: 100,
      quota: null,
    },
  ]

  it('shows correct counts on the All, At risk, and No quota chips', async () => {
    setupApiSuccess(reposWithQuota)
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const chips = wrapper.findAll('.quota-fchip')
    expect(chips[0]!.text()).toContain('All')
    expect(chips[0]!.text()).toContain('3')
    expect(chips[1]!.text()).toContain('At risk')
    expect(chips[1]!.text()).toContain('1')
    expect(chips[2]!.text()).toContain('No quota')
    expect(chips[2]!.text()).toContain('1')
  })

  it('filters to only at-risk repos when the At risk chip is clicked', async () => {
    setupApiSuccess(reposWithQuota)
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const chips = wrapper.findAll('.quota-fchip')
    await chips[1]!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('over-warn')
    expect(wrapper.text()).not.toContain('healthy')
    expect(wrapper.text()).not.toContain('unconfigured')
  })

  it('filters to only unconfigured repos when the No quota chip is clicked', async () => {
    setupApiSuccess(reposWithQuota)
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const chips = wrapper.findAll('.quota-fchip')
    await chips[2]!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('unconfigured')
    expect(wrapper.text()).not.toContain('over-warn')
    expect(wrapper.text()).not.toContain('healthy')
  })

  it('renders a quota meter on a card with a configured quota', async () => {
    setupApiSuccess(reposWithQuota)
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const cards = wrapper.findAll('.repo-card')
    const overWarnCard = cards.find((c) => c.text().includes('over-warn'))
    expect(overWarnCard!.find('.quota-meter').exists()).toBe(true)
    expect(overWarnCard!.text()).toContain('Warning')

    const unconfiguredCard = cards.find((c) => c.text().includes('unconfigured'))
    expect(unconfiguredCard!.find('.quota-meter').exists()).toBe(false)
  })
})

describe('ReposView group by host', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('groups repos sharing an ssh_host under one pool header', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const groupButton = wrapper.findAll('button').find((b) => b.text() === 'Group by host')
    await groupButton!.trigger('click')
    await flushPromises()

    const headers = wrapper.findAll('.pool-header')
    expect(headers).toHaveLength(1)
    expect(headers[0]!.text()).toContain('backup.example.com')
    expect(headers[0]!.text()).toContain('3 repos')

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).toContain('media-weekly')
  })

  it('is mutually exclusive with group by tag', async () => {
    setupApiSuccess()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/stats') return Promise.resolve({ data: mockRepos })
      if (url === '/repo-tags') return Promise.resolve({ data: [] })
      if (String(url).startsWith('/tags')) {
        return Promise.resolve({ data: [{ id: 1, name: 'critical', color: '#ff0000' }] })
      }
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const tagButton = wrapper.findAll('button').find((b) => b.text() === 'Group by tag')
    const hostButton = wrapper.findAll('button').find((b) => b.text() === 'Group by host')

    await hostButton!.trigger('click')
    await flushPromises()
    expect(hostButton!.classes()).toContain('active')
    expect(tagButton!.classes()).not.toContain('active')
    expect(wrapper.find('.pool-header').exists()).toBe(true)

    await tagButton!.trigger('click')
    await flushPromises()
    expect(tagButton!.classes()).toContain('active')
    expect(hostButton!.classes()).not.toContain('active')
    expect(wrapper.find('.pool-header').exists()).toBe(false)
  })

  it('dims filtered-out repos instead of removing them from the group', async () => {
    const repos: RepoWithStats[] = [
      {
        ...baseRepo,
        id: 1,
        name: 'at-risk-repo',
        repo_path: '/backup/at-risk-repo',
        total_deduplicated_size: 600,
        quota: {
          warn_bytes: 500,
          critical_bytes: 1000,
          warn_action: 'notify_only',
          critical_action: 'block_backups',
          enabled: true,
        },
      },
      {
        ...baseRepo,
        id: 2,
        name: 'healthy-repo',
        repo_path: '/backup/healthy-repo',
        total_deduplicated_size: 100,
        quota: null,
      },
    ]
    setupApiSuccess(repos)
    const wrapper = renderWithPlugins(ReposView, {
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const hostButton = wrapper.findAll('button').find((b) => b.text() === 'Group by host')
    await hostButton!.trigger('click')
    await flushPromises()

    const atRiskChip = wrapper.findAll('.quota-fchip').find((c) => c.text().includes('At risk'))
    await atRiskChip!.trigger('click')
    await flushPromises()

    // Both cards still render (the group keeps its full picture)...
    const cards = wrapper.findAll('.repo-card')
    expect(cards).toHaveLength(2)
    // ...but only the one that doesn't match the filter is dimmed.
    const healthyCard = cards.find((c) => c.text().includes('healthy-repo'))
    expect(healthyCard!.classes()).toContain('repo-card-dim')
    const atRiskCard = cards.find((c) => c.text().includes('at-risk-repo'))
    expect(atRiskCard!.classes()).not.toContain('repo-card-dim')
  })
})
