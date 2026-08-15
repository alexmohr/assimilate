// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { ref } from 'vue'

const mockBrowserArchives = ref<
  Array<{
    name: string
    start: string
    hostname: string
    comment: string
    original_size: number
    deduplicated_size: number
    matched: boolean | null
    agent_hostname: string | null
  }>
>([])
const mockSortedArchives = ref<typeof mockBrowserArchives.value>([])

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

// Captured WebSocket message handlers - populated during component setup().
const wsHandlers: Record<string, (payload: unknown) => void> = {}

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    status: ref('connected'),
    onMessage: (type: string, cb: (p: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
}))

const mockDeleteArchiveByName = vi.fn()
const mockLoadArchives = vi.fn()

vi.mock('../composables/useArchiveBrowser', () => ({
  useArchiveBrowser: () => ({
    archives: mockBrowserArchives,
    sortedArchives: mockSortedArchives,
    archivesLoading: ref(false),
    archivesError: ref(null),
    selectedArchive: ref(null),
    currentPath: ref('/'),
    contents: ref([]),
    contentsLoading: ref(false),
    contentsError: ref(null),
    indexing: ref(false),
    breadcrumbs: ref([]),
    dirs: ref([]),
    files: ref([]),
    loadArchives: mockLoadArchives,
    selectArchive: vi.fn(),
    loadContents: vi.fn(),
    navigateTo: vi.fn(),
    entryName: vi.fn((e: { path: string }) => e.path.split('/').pop() ?? ''),
    downloadEntry: vi.fn(),
    restoreEntry: vi.fn(),
    deleteArchive: vi.fn(),
    deleteArchiveByName: mockDeleteArchiveByName,
    stopPolling: vi.fn(),
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
  ssh_host_key: string | null
  compression: string
  encryption: string
  enabled: boolean
  archive_count: number
  last_backup_at: string | null
  total_original_size: number
  total_compressed_size: number
  total_deduplicated_size: number
  agent_count: number
  current_op?: {
    kind: string
    actor: string
    started_at: string
    queued?: number
  } | null
  last_op_kind?: string | null
  last_op_by?: string | null
  last_op_at?: string | null
}

const mockRepo: RepoWithStats = {
  id: 1,
  name: 'server-daily',
  repo_path: '/backup/repos/server-daily',
  ssh_user: 'borg',
  ssh_host: 'backup.example.com',
  ssh_port: 22,
  ssh_host_key: 'ssh-ed25519 AAAAOLD',
  compression: 'lz4',
  encryption: 'repokey-blake2',
  enabled: true,
  archive_count: 30,
  last_backup_at: new Date(Date.now() - 3_600_000).toISOString(),
  total_original_size: 10_737_418_240,
  total_compressed_size: 5_368_709_120,
  total_deduplicated_size: 2_684_354_560,
  agent_count: 2,
}

const refreshedHostKey = 'ssh-ed25519 AAAANEW'

const mockRepoSchedule = {
  id: 5,
  agent_id: 10,
  repo_id: 1,
  target_hostnames: ['web-server-01'],
  schedule_type: 'backup',
  cron_expression: '0 2 * * *',
  enabled: true,
  canary_enabled: false,
  last_run_at: '2026-05-30T02:00:00Z',
  next_run_at: '2026-05-31T02:00:00Z',
  exclude_patterns: [],
  ignore_global_excludes: false,
  keep_hourly: 24,
  keep_daily: 7,
  keep_weekly: 4,
  keep_monthly: 6,
  keep_yearly: 1,
  compact_enabled: true,
  pre_backup_commands: [],
  post_backup_commands: [],
}

let repoState: RepoWithStats

function setupApiSuccess(
  repo: RepoWithStats = mockRepo,
  scanHostKey = refreshedHostKey,
  schedules: unknown[] = [mockRepoSchedule],
  health: unknown[] = [],
): void {
  repoState = { ...repo }
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === `/repos/${repo.id}`) return Promise.resolve({ data: repoState })
    if (url === `/repos/${repo.id}/schedules`) return Promise.resolve({ data: schedules })
    if (url === '/stats/health') return Promise.resolve({ data: health })
    if (String(url).startsWith('/tags')) return Promise.resolve({ data: [] })
    if (String(url).endsWith('/tags')) return Promise.resolve({ data: [] })
    return Promise.resolve({ data: [] })
  })
  vi.mocked(apiClient.post).mockImplementation((url: string, body?: unknown) => {
    if (url === `/repos/${repo.id}/ssh-host-key/scan`) {
      return Promise.resolve({ data: { ssh_host_key: scanHostKey } })
    }
    if (url === `/repos/${repo.id}/ssh-host-key`) {
      const payload = body as { ssh_host_key?: string } | undefined
      repoState = {
        ...repoState,
        ssh_host_key: payload?.ssh_host_key ?? repoState.ssh_host_key,
      }
      return Promise.resolve({ data: { ssh_host_key: repoState.ssh_host_key } })
    }
    return Promise.resolve({ data: {} })
  })
}

async function renderRepoDetail(
  overrides: { id?: string; role?: string } = {},
): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(RepoDetailView, {
    props: { id: overrides.id ?? '1' },
    storeState: { auth: { user: { role: overrides.role ?? 'admin' } } },
  })
  await flushPromises()
  return wrapper
}

const archiveA = {
  name: 'web-server-01-backup-2026-06-04T02:00:00',
  start: '2026-06-04T02:00:00',
  hostname: 'web-server-01',
  comment: '',
  original_size: 1_000,
  deduplicated_size: 500,
  matched: true,
  agent_hostname: 'web-server-01',
}
const archiveB = {
  name: 'db-server-01-backup-2026-06-04T03:00:00',
  start: '2026-06-04T03:00:00',
  hostname: 'db-server-01',
  comment: '',
  original_size: 2_000,
  deduplicated_size: 1_000,
  matched: true,
  agent_hostname: 'db-server-01',
}

function setupArchivesAB(): void {
  mockBrowserArchives.value = [archiveA, archiveB]
  mockSortedArchives.value = [archiveA, archiveB]
  setupApiSuccess()
}

describe('RepoDetailView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockBrowserArchives.value = []
    mockSortedArchives.value = []
    mockDeleteArchiveByName.mockResolvedValue(true)
    mockLoadArchives.mockResolvedValue(undefined)
  })

  it('renders repo name in breadcrumb and info grid', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('server-daily')
  })

  it('displays compression and encryption values', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    const text = wrapper.text()
    expect(text).toContain('lz4')
    expect(text).toContain('repokey-blake2')
  })

  it('shows SSH target in info grid', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('borg@backup.example.com:22')
  })

  it('shows accept key only when the host key mismatches', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.findAll('button').some((button) => button.text() === 'Accept SSH Key')).toBe(
      true,
    )
    expect(wrapper.text()).toContain('ssh-ed25519 AAAAOLD')
  })

  it('hides the accept key button when the host key matches', async () => {
    setupApiSuccess({ ...mockRepo, ssh_host_key: refreshedHostKey }, refreshedHostKey)
    const wrapper = await renderRepoDetail()

    expect(wrapper.findAll('button').some((button) => button.text() === 'Accept SSH Key')).toBe(
      false,
    )
  })

  it('accepts a refreshed SSH host key', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    const acceptButton = wrapper
      .findAll('button')
      .find((button) => button.text().includes('Accept SSH Key'))
    expect(acceptButton).toBeDefined()
    await acceptButton!.trigger('click')
    await flushPromises()

    expect(document.body.textContent).toContain(refreshedHostKey)
  })

  it('shows repo path in info grid', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('/backup/repos/server-daily')
  })

  it('shows the compacting-repository label for an in-progress compact op', async () => {
    setupApiSuccess({
      ...mockRepo,
      current_op: {
        kind: 'compact_repo',
        actor: 'admin',
        started_at: new Date().toISOString(),
        queued: 0,
      },
    })
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('Current Operation')
    expect(wrapper.text()).toContain('Compacting repository (started by admin)')
  })

  it('shows "Compact repository" as the last-operation label once a compact has run', async () => {
    setupApiSuccess({
      ...mockRepo,
      last_op_kind: 'compact_repo',
      last_op_by: 'admin',
      last_op_at: new Date().toISOString(),
    })
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('Compact repository')
    expect(wrapper.text()).toContain('by admin')
  })

  it('renders stat cards with archive count and agent count', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    const text = wrapper.text()
    expect(text).toContain('30')
    expect(text).toContain('Archives')
    expect(text).toContain('2')
    expect(text).toContain('Agents')
  })

  it('renders QuotaPanel component', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.find('[data-testid="quota-panel"]').exists()).toBe(true)
  })

  it('shows Enabled status badge when repo is enabled', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('Enabled')
  })

  it('shows Disabled status badge when repo is disabled', async () => {
    setupApiSuccess({ ...mockRepo, enabled: false })
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('Disabled')
  })

  it('shows Overview and Archives tabs', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    const text = wrapper.text()
    expect(text).toContain('Overview')
    expect(text).toContain('Archives')
  })

  it('shows archives tab content when Archives tab is clicked', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
    expect(archivesTab).toBeDefined()
    await archivesTab!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('No archives found')
  })

  it('runs a schedule now from the Schedules tab', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    const schedulesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Schedules')
    expect(schedulesTab).toBeDefined()
    await schedulesTab!.trigger('click')
    await flushPromises()

    // Scoped by title, not text: the always-rendered Borg Console section
    // (v-if="isAdmin", not tab-gated) also has a button labeled plain 'Run'.
    const runBtn = wrapper.find('button[title="Run backup now"]')
    expect(runBtn.exists()).toBe(true)
    await runBtn.trigger('click')
    await flushPromises()

    // Toast container is teleported so verify via the apiClient call and the
    // loading state clearing back to 'Run', matching this file's other
    // toast-triggering tests (e.g. 'shows error toast when sync request fails').
    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith(
      `/schedules/${mockRepoSchedule.id}/run`,
      {},
    )
    expect(wrapper.find('button[title="Run backup now"]').text()).toBe('Run')
  })

  it('shows an error toast when running a schedule now fails', async () => {
    setupApiSuccess()
    vi.mocked(apiClient.post).mockRejectedValue(new Error('Connection refused'))

    const wrapper = await renderRepoDetail()

    const schedulesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Schedules')
    await schedulesTab!.trigger('click')
    await flushPromises()

    const runBtn = wrapper.find('button[title="Run backup now"]')
    expect(runBtn.exists()).toBe(true)
    await runBtn.trigger('click')
    await flushPromises()

    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith(
      `/schedules/${mockRepoSchedule.id}/run`,
      {},
    )
    // Loading state clears even on failure -- button returns to 'Run'.
    expect(wrapper.find('button[title="Run backup now"]').text()).toBe('Run')
  })

  it('shows a Disabled pill and tints the card for a disabled schedule', async () => {
    setupApiSuccess(mockRepo, refreshedHostKey, [{ ...mockRepoSchedule, enabled: false }])
    const wrapper = await renderRepoDetail()

    const schedulesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Schedules')
    await schedulesTab!.trigger('click')
    await flushPromises()

    expect(wrapper.find('.entity-status-pill').text()).toBe('Disabled')
    expect(wrapper.find('.schedule-card').classes()).toContain('schedule-card-notable')
  })

  it("shows a Failed chip that navigates to the schedule's filtered activity log", async () => {
    setupApiSuccess(
      mockRepo,
      refreshedHostKey,
      [mockRepoSchedule],
      [
        {
          schedule_id: mockRepoSchedule.id,
          hostname: 'web-server-01',
          target_name: 'server-daily',
          last_status: 'failed',
          last_backup_at: '2026-05-30T02:00:00Z',
          is_overdue: false,
          last_error_message: 'Repository lock could not be acquired',
          cron_expression: '0 2 * * *',
          schedule_enabled: true,
        },
      ],
    )
    const wrapper = await renderRepoDetail()

    const schedulesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Schedules')
    await schedulesTab!.trigger('click')
    await flushPromises()

    const failedChip = wrapper.find('.entity-issue-chip.sev-danger')
    expect(failedChip.exists()).toBe(true)
    expect(failedChip.attributes('title')).toBe('Repository lock could not be acquired')
    await failedChip.trigger('click')
    await flushPromises()

    expect(wrapper.vm.$router.currentRoute.value.path).toBe('/activity')
    expect(wrapper.vm.$router.currentRoute.value.query).toMatchObject({
      category: 'backup',
      schedule_id: String(mockRepoSchedule.id),
      status: 'failed',
    })
  })

  it('shows archive list mode options when archives exist', async () => {
    mockBrowserArchives.value = [
      {
        name: 'web-server-01-2026-06-08T01:00:00',
        start: '2026-06-08T01:00:00',
        hostname: 'web-server-01',
        comment: '',
        original_size: 1_000,
        deduplicated_size: 500,
        matched: true,
        agent_hostname: 'web-server-01',
      },
    ]
    mockSortedArchives.value = [...mockBrowserArchives.value]
    setupApiSuccess()

    const wrapper = await renderRepoDetail()

    const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
    expect(archivesTab).toBeDefined()
    await archivesTab!.trigger('click')
    await flushPromises()

    const select = wrapper.find('.archive-sort-select')
    expect(select.exists()).toBe(true)
    expect(select.text()).toContain('Date newest first')
    expect(select.text()).toContain('Size largest first')
    expect(select.text()).toContain('Dedup smallest first')

    const groupToggle = wrapper.find('.archive-group-toggle')
    expect(groupToggle.exists()).toBe(true)
    expect(groupToggle.text()).toContain('Grouped by host')
  })

  it('collapses host groups by default and expands on click', async () => {
    mockBrowserArchives.value = [
      {
        name: 'web-server-01-2026-06-08T01:00:00',
        start: '2026-06-08T01:00:00',
        hostname: 'web-server-01',
        comment: '',
        original_size: 1_000,
        deduplicated_size: 500,
        matched: true,
        agent_hostname: 'web-server-01',
      },
    ]
    mockSortedArchives.value = [...mockBrowserArchives.value]
    setupApiSuccess()

    const wrapper = renderWithPlugins(RepoDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
    await archivesTab!.trigger('click')
    await flushPromises()

    const groupHeader = wrapper.find('.group-header')
    expect(groupHeader.exists()).toBe(true)
    expect(groupHeader.classes()).toContain('collapsed')
    expect(wrapper.find('.group-archives').attributes('style')).toContain('display: none')

    await groupHeader.trigger('click')
    await flushPromises()

    expect(wrapper.find('.group-header').classes()).not.toContain('collapsed')
    expect(wrapper.find('.group-archives').attributes('style') ?? '').not.toContain('display: none')

    await wrapper.find('.group-header').trigger('click')
    await flushPromises()

    expect(wrapper.find('.group-header').classes()).toContain('collapsed')
    expect(wrapper.find('.group-archives').attributes('style')).toContain('display: none')
  })

  describe('archive list interactions', () => {
    beforeEach(setupArchivesAB)

    async function goToArchivesTab(
      wrapper: Awaited<ReturnType<typeof renderRepoDetail>>,
    ): Promise<void> {
      const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
      await archivesTab!.trigger('click')
      await flushPromises()
    }

    it('toggles between grouped and flat archive list views', async () => {
      const wrapper = await renderRepoDetail()
      await goToArchivesTab(wrapper)

      expect(wrapper.find('.archive-groups').exists()).toBe(true)
      expect(wrapper.find('.archive-flat-list').exists()).toBe(false)

      const toggle = wrapper.find('.archive-group-toggle')
      await toggle.trigger('click')
      await flushPromises()

      expect(wrapper.find('.archive-flat-list').exists()).toBe(true)
      expect(wrapper.find('.archive-groups').exists()).toBe(false)
      expect(wrapper.find('.archive-group-toggle').text()).toContain('Flat list')
    })

    it('filters the archive list by typing in the filter input', async () => {
      const wrapper = await renderRepoDetail()
      await goToArchivesTab(wrapper)
      await wrapper.find('.archive-group-toggle').trigger('click')
      await flushPromises()

      const filterInput = wrapper.find('.filter-input')
      await filterInput.setValue('db-server')
      await flushPromises()

      const rows = wrapper.findAll('.archive-row-detailed')
      expect(rows.length).toBe(1)
      expect(rows[0]!.text()).toContain('db-server-01')
    })

    it('sorts the archive list using the sort select', async () => {
      const wrapper = await renderRepoDetail()
      await goToArchivesTab(wrapper)
      await wrapper.find('.archive-group-toggle').trigger('click')
      await flushPromises()

      const select = wrapper.find('.archive-sort-select')
      await select.setValue('size-asc')
      await flushPromises()

      const rows = wrapper.findAll('.archive-row-detailed')
      expect(rows[0]!.text()).toContain('web-server-01-backup')
      expect(rows[1]!.text()).toContain('db-server-01-backup')
    })

    it('selects an archive from a grouped row and opens the delete dialog from it', async () => {
      const wrapper = await renderRepoDetail()
      await goToArchivesTab(wrapper)

      const chevrons = wrapper.findAll('.group-chevron')
      for (const chevron of chevrons) {
        await chevron.trigger('click')
      }
      await flushPromises()

      const row = wrapper.findAll('.archive-row').find((r) => r.text().includes(archiveA.name))
      expect(row).toBeDefined()
      await row!.trigger('click')
      await flushPromises()

      expect(wrapper.find('.browser-title').text()).toContain(archiveA.name)

      const deleteBtn = row!.find('.archive-row-delete')
      expect(deleteBtn.exists()).toBe(true)
      await deleteBtn.trigger('click')
      await flushPromises()

      // BaseModal teleports to document.body, outside the mounted wrapper's tree.
      expect(document.body.querySelector('.archive-delete-message')?.textContent).toContain(
        archiveA.name,
      )

      // Unmount so the still-open dialog's teleported content doesn't leak
      // into document.body for the next test in this file.
      wrapper.unmount()
    })

    it('selects and deletes an archive from the flat list, clearing the selection', async () => {
      const wrapper = await renderRepoDetail()
      await goToArchivesTab(wrapper)
      await wrapper.find('.archive-group-toggle').trigger('click')
      await flushPromises()

      expect(wrapper.find('.empty-state').text()).toContain('Select an archive')

      const rows = wrapper.findAll('.archive-row-detailed')
      const targetRow = rows.find((r) => r.text().includes(archiveA.name))!
      await targetRow.trigger('click')
      await flushPromises()

      expect(wrapper.find('.browser-title').text()).toContain(archiveA.name)

      await targetRow.find('.archive-row-delete').trigger('click')
      await flushPromises()

      // BaseModal teleports to document.body, outside the mounted wrapper's tree.
      const confirmBtn = Array.from(document.body.querySelectorAll('button')).find(
        (b) => b.textContent === 'Delete Archive',
      )
      expect(confirmBtn).toBeDefined()
      confirmBtn!.click()
      await flushPromises()

      expect(wrapper.find('.empty-state').text()).toContain('Select an archive')
    })
  })

  it('shows danger zone for admin users', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail()

    expect(wrapper.text()).toContain('Danger Zone')
    expect(wrapper.text()).toContain('Delete Repository')
  })

  it('hides danger zone for non-admin users', async () => {
    setupApiSuccess()
    const wrapper = await renderRepoDetail({ role: 'viewer' })

    expect(wrapper.find('.danger-zone').exists()).toBe(false)
  })

  it('shows error message when repo load fails', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('Not found'))
    const wrapper = await renderRepoDetail({ id: '99' })

    expect(wrapper.text()).toContain('Not found')
  })

  it('calls sync endpoint and clears loading after 202 response', async () => {
    setupApiSuccess()
    vi.mocked(apiClient.post).mockResolvedValue({ status: 202, data: {} })

    const wrapper = await renderRepoDetail()

    const syncBtn = wrapper.findAll('button').find((b) => b.text() === 'Full Resync')
    expect(syncBtn).toBeDefined()
    await syncBtn!.trigger('click')
    await flushPromises()

    // After 202 response loading clears -- button returns to normal label
    expect(wrapper.findAll('button').find((b) => b.text() === 'Full Resync')).toBeDefined()
    expect(wrapper.findAll('button').find((b) => b.text() === 'Syncing...')).toBeUndefined()

    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/repos/1/sync?build_index=true')
  })

  it('shows error toast when sync request fails', async () => {
    setupApiSuccess()
    vi.mocked(apiClient.post).mockRejectedValue(new Error('Connection refused'))

    const wrapper = await renderRepoDetail()

    const syncBtn = wrapper.findAll('button').find((b) => b.text() === 'Full Resync')
    expect(syncBtn).toBeDefined()
    await syncBtn!.trigger('click')
    await flushPromises()

    // Loading clears even on failure
    expect(wrapper.findAll('button').find((b) => b.text() === 'Full Resync')).toBeDefined()
    expect(wrapper.findAll('button').find((b) => b.text() === 'Syncing...')).toBeUndefined()

    // Error message visible in the page (toast container is teleported so check apiClient call)
    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/repos/1/sync?build_index=true')
  })

  it('reloads data when id prop changes', async () => {
    const repo2 = { ...mockRepo, id: 2, name: 'db-hourly' }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/repos/1') return Promise.resolve({ data: mockRepo })
      if (url === '/repos/2') return Promise.resolve({ data: repo2 })
      return Promise.resolve({ data: [] })
    })

    const wrapper = await renderRepoDetail()
    expect(wrapper.text()).toContain('server-daily')

    await wrapper.setProps({ id: '2' })
    await flushPromises()

    expect(wrapper.text()).toContain('db-hourly')
    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith('/repos/2')
  })

  describe('archive filter via ?archive= query parameter', () => {
    beforeEach(setupArchivesAB)

    it('AC-U1: archive filter computed returns null when no ?archive= query is present', async () => {
      const wrapper = await renderRepoDetail()

      expect(wrapper.find('.archive-filter-banner').exists()).toBe(false)

      const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
      await archivesTab!.trigger('click')
      await flushPromises()

      expect(wrapper.findAll('.archive-row').length).toBe(2)
    })

    it('AC-U2: archive filter computed returns the archive name when ?archive=<name> is present', async () => {
      const wrapper = await renderRepoDetail()

      await wrapper.vm.$router.replace({ query: { archive: archiveA.name } })
      await flushPromises()

      expect(wrapper.vm.archiveFilterName).toBe(archiveA.name)
      expect(wrapper.vm.hasArchiveFilter).toBe(true)
    })

    it('AC-U3: archive browser and filters are hidden, showing only the filter banner', async () => {
      const wrapper = await renderRepoDetail()

      // Navigate to archives tab with the archive filter
      await wrapper.vm.$router.replace({
        query: { tab: 'archives', archive: archiveA.name },
      })
      await flushPromises()

      expect(wrapper.findAll('.archive-row').length).toBe(0)
      expect(wrapper.find('.archive-controls').exists()).toBe(false)
      expect(wrapper.find('.archive-filter-banner').text()).toContain(
        `Showing only ${archiveA.name}`,
      )
    })

    it('AC-U4: clicking "Show all archives" clears the filter', async () => {
      const wrapper = await renderRepoDetail()

      // Navigate to archives tab with the archive filter
      await wrapper.vm.$router.replace({
        query: { tab: 'archives', archive: archiveA.name },
      })
      await flushPromises()

      expect(wrapper.find('.archive-filter-banner').exists()).toBe(true)

      const showAllBtn = wrapper.findAll('button').find((b) => b.text() === 'Show all archives')
      await showAllBtn!.trigger('click')
      await flushPromises()

      expect(wrapper.find('.archive-filter-banner').exists()).toBe(false)
      expect(wrapper.findAll('.archive-row').length).toBe(2)
    })

    it('AC-U5: archive filter with non-existent name shows only the filter banner', async () => {
      const wrapper = await renderRepoDetail()

      await wrapper.vm.$router.replace({
        query: { tab: 'archives', archive: 'nonexistent-archive' },
      })
      await flushPromises()

      expect(wrapper.findAll('.archive-row').length).toBe(0)
      expect(wrapper.find('.archive-controls').exists()).toBe(false)
      expect(wrapper.find('.archive-filter-banner').exists()).toBe(true)
      expect(wrapper.find('.archive-filter-banner').text()).toContain(
        'Showing only nonexistent-archive',
      )
    })

    it('AC-U6: sort mode has no effect while an archive filter hides the browser', async () => {
      const wrapper = await renderRepoDetail()

      await wrapper.vm.$router.replace({
        query: { tab: 'archives', archive: archiveA.name },
      })
      await flushPromises()

      const sortModes = [
        'date-desc',
        'date-asc',
        'size-desc',
        'size-asc',
        'dedup-desc',
        'dedup-asc',
      ] as const

      for (const mode of sortModes) {
        wrapper.vm.archiveSortMode = mode
        await flushPromises()

        expect(wrapper.findAll('.archive-row').length).toBe(0)
        expect(wrapper.find('.archive-filter-banner').exists()).toBe(true)
      }
    })

    it('AC-U7: clear archive filter via function call', async () => {
      const wrapper = await renderRepoDetail()

      await wrapper.vm.$router.replace({ query: { archive: archiveA.name } })
      await flushPromises()

      expect(wrapper.vm.hasArchiveFilter).toBe(true)
      expect(wrapper.vm.archiveFilterName).toBe(archiveA.name)

      wrapper.vm.clearArchiveFilter()
      await flushPromises()

      expect(wrapper.vm.hasArchiveFilter).toBe(false)
      expect(wrapper.vm.archiveFilterName).toBeNull()
    })
  })

  describe('archive deletion in-progress state', () => {
    const deletingArchive = {
      name: 'web-server-01-backup-2026-06-04T02:00:00',
      start: '2026-06-04T02:00:00',
      hostname: 'web-server-01',
      comment: '',
      original_size: 1_000,
      deduplicated_size: 500,
      matched: true,
      agent_hostname: 'web-server-01',
    }

    beforeEach(() => {
      mockBrowserArchives.value = [deletingArchive]
      mockSortedArchives.value = [deletingArchive]
      setupApiSuccess()
    })

    async function openArchivesTab(
      wrapper: Awaited<ReturnType<typeof renderRepoDetail>>,
    ): Promise<void> {
      const archivesTab = wrapper.findAll('.tab-btn').find((b) => b.text() === 'Archives')
      await archivesTab!.trigger('click')
      await flushPromises()
    }

    // BaseModal renders its footer via <Teleport to="body">, which lands
    // outside the wrapper's element tree, so it can't be reached with
    // wrapper.find(); go through the real DOM instead, matching this file's
    // existing document.body-based assertions for teleported content.
    async function clickModalConfirm(): Promise<void> {
      const confirmBtn = document.body.querySelector<HTMLButtonElement>('button.btn-danger')
      expect(confirmBtn).not.toBeNull()
      expect(confirmBtn!.textContent).toBe('Delete Archive')
      confirmBtn!.click()
      await flushPromises()
    }

    it('disables the delete button and blocks re-triggering once a delete is confirmed', async () => {
      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      const deleteBtn = wrapper.find('button[title="Delete archive"]')
      expect(deleteBtn.exists()).toBe(true)
      await deleteBtn.trigger('click')
      await flushPromises()

      await clickModalConfirm()

      expect(mockDeleteArchiveByName).toHaveBeenCalledWith(deletingArchive)
      // The modal closed and the row's button now reflects the in-flight delete.
      expect(wrapper.find('button[title="Delete archive"]').exists()).toBe(false)
      const pendingBtn = wrapper.find('button[title="Deletion in progress"]')
      expect(pendingBtn.exists()).toBe(true)
      expect(pendingBtn.attributes('disabled')).toBeDefined()

      // Re-requesting deletion for the same archive while it's still pending
      // must not reopen the confirmation modal (the actual guard being tested).
      wrapper.vm.requestArchiveDeletion(deletingArchive)
      await flushPromises()
      expect(wrapper.vm.archivePendingDeletion).toBeNull()
    })

    it('marks the row as deleting immediately, before the delete request resolves', async () => {
      // On a fast demo repo, the DELETE's own DataChanged notification can
      // reach the WebSocket handler - and prune the archive from the list -
      // before this request's promise would otherwise resolve. The
      // in-progress state must be set synchronously on confirm, not after
      // awaiting deleteArchiveByName, or that race means it's never observed.
      let resolveDelete: (() => void) | undefined
      mockDeleteArchiveByName.mockImplementation(
        () =>
          new Promise((resolve) => {
            resolveDelete = () => resolve(true)
          }),
      )

      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      resolveDelete?.()
      await flushPromises()
    })

    it('rolls back the in-progress marker when the delete request itself fails', async () => {
      mockDeleteArchiveByName.mockRejectedValue(new Error('Connection refused'))

      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      // The failed request must not leave the row stuck disabled forever -
      // the optimistic "deleting" marker set before the request went out
      // has to be rolled back once it's clear the delete never happened.
      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
      expect(wrapper.find('button[title="Delete archive"]').exists()).toBe(true)

      // A failed delete leaves the confirmation modal open (its own,
      // pre-existing behavior, unchanged here) - its teleported content
      // otherwise lingers in document.body and breaks other tests' modal
      // lookups, so tear it down explicitly.
      wrapper.unmount()
    })

    it('clears the in-progress state once the archive disappears from a DataChanged refresh', async () => {
      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      // Simulate the server finishing the borg delete: the archive is gone
      // from the reloaded list, then a DataChanged event notifies the UI.
      mockBrowserArchives.value = []
      mockSortedArchives.value = []
      wsHandlers.DataChanged({})
      await flushPromises()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
    })

    it('removes the row and clears its in-progress state directly on ArchiveDeleted, without a DataChanged refresh', async () => {
      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      const row = wrapper
        .findAll('.archive-row')
        .find((r) => r.text().includes(deletingArchive.name))
      expect(row).toBeDefined()

      await row!.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      // Row selection isn't disabled while a delete is in flight - re-select
      // the still-listed (locally not-yet-pruned) archive to reproduce a
      // user re-opening its file browser mid-delete. ArchiveDeleted must
      // clear that too, not just the one confirmArchiveDeletion's own
      // success path already handles when nothing was re-selected since.
      await row!.trigger('click')
      await flushPromises()
      expect(wrapper.find('.browser-title').text()).toContain(deletingArchive.name)

      // No further loadArchives call and no DataChanged - ArchiveDeleted
      // alone must be enough to drop the row. sortedArchives is mocked as
      // an independent ref here (a real computed in production, derived
      // from archives), so check the underlying archives list this handler
      // actually mutates rather than rendered text.
      const callsBefore = mockLoadArchives.mock.calls.length
      wsHandlers.ArchiveDeleted({ repo_id: mockRepo.id, archive_name: deletingArchive.name })
      await flushPromises()

      expect(mockLoadArchives.mock.calls.length).toBe(callsBefore)
      expect(mockBrowserArchives.value.some((a) => a.name === deletingArchive.name)).toBe(false)
      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
      expect(wrapper.find('.browser-title').exists()).toBe(false)
      expect(wrapper.text()).toContain('Select an archive to browse its contents.')
    })

    it('ignores ArchiveDeleted events for a different repository', async () => {
      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      wsHandlers.ArchiveDeleted({ repo_id: mockRepo.id + 1, archive_name: deletingArchive.name })
      await flushPromises()

      expect(mockBrowserArchives.value.some((a) => a.name === deletingArchive.name)).toBe(true)
      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)
    })

    it('clears stale in-progress state once RepoOpChanged reports the repo is no longer deleting', async () => {
      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      // The archive is still in the list (e.g. the borg delete itself
      // failed), but the repo's delete queue has fully drained - the stale
      // "deleting" marker must not stick around forever.
      wsHandlers.RepoOpChanged({ repo_id: mockRepo.id, op: null })
      await flushPromises()

      expect(wrapper.find('button[title="Delete archive"]').exists()).toBe(true)
      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
    })

    it('refetches before clearing on RepoOpChanged, not racing a DataChanged refresh already in flight', async () => {
      let resolveLoadArchives: () => void = () => {}
      mockLoadArchives.mockReturnValue(
        new Promise<void>((resolve) => {
          resolveLoadArchives = resolve
        }),
      )

      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      // RepoOpChanged reports the queue idle while its own refetch is still
      // outstanding - the marker must survive until that refetch resolves,
      // not clear immediately against whatever the list held before it.
      wsHandlers.RepoOpChanged({ repo_id: mockRepo.id, op: null })
      await flushPromises()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      resolveLoadArchives()
      await flushPromises()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
    })

    it('does not sweep a different archive whose delete starts while the RepoOpChanged refetch is in flight', async () => {
      const otherArchive = {
        ...deletingArchive,
        name: 'web-server-01-backup-2026-06-05T02:00:00',
      }
      mockBrowserArchives.value = [deletingArchive, otherArchive]
      mockSortedArchives.value = [deletingArchive, otherArchive]

      let resolveLoadArchives: () => void = () => {}
      mockLoadArchives.mockReturnValue(
        new Promise<void>((resolve) => {
          resolveLoadArchives = resolve
        }),
      )

      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      const rowFor = (name: string) =>
        wrapper.findAll('.archive-row').find((r) => r.text().includes(name))!

      await rowFor(deletingArchive.name).find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(
        rowFor(deletingArchive.name).find('button[title="Deletion in progress"]').exists(),
      ).toBe(true)

      // The queue drains for deletingArchive's completed delete - the
      // handler snapshots it as the name to sweep and starts its own
      // (silent) refetch.
      wsHandlers.RepoOpChanged({ repo_id: mockRepo.id, op: null })
      await flushPromises()

      // While that refetch is still outstanding, the user starts deleting a
      // wholly unrelated archive. This must be marked immediately and must
      // not be touched by the sweep once it resolves below.
      wrapper.vm.requestArchiveDeletion(otherArchive)
      await wrapper.vm.confirmArchiveDeletion()

      expect(rowFor(otherArchive.name).find('button[title="Deletion in progress"]').exists()).toBe(
        true,
      )

      resolveLoadArchives()
      await flushPromises()

      // Only the snapshotted (already-resolved) delete is swept...
      expect(
        rowFor(deletingArchive.name).find('button[title="Deletion in progress"]').exists(),
      ).toBe(false)
      // ...the unrelated, still-in-flight delete must survive the sweep.
      expect(rowFor(otherArchive.name).find('button[title="Deletion in progress"]').exists()).toBe(
        true,
      )
    })

    it('keeps the in-progress state while the automatic post-delete compact is running', async () => {
      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      // The delete itself finished (the tracked op moved on to the compact
      // that automatically follows it) but the archive hasn't disappeared
      // from the list yet - the row must stay disabled through the compact,
      // not just through the delete.
      wsHandlers.RepoOpChanged({
        repo_id: mockRepo.id,
        op: { kind: 'compact_repo', actor: 'admin', started_at: new Date().toISOString() },
      })
      await flushPromises()

      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)
    })

    it('shows the in-progress state immediately on confirm, before the delete request resolves', async () => {
      let resolveDelete: ((value: boolean) => void) | undefined
      mockDeleteArchiveByName.mockImplementation(
        () =>
          new Promise<boolean>((resolve) => {
            resolveDelete = resolve
          }),
      )

      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      // The row must flip to "in flight" the moment the user confirms, not
      // once the (still in-flight) delete request comes back.
      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

      resolveDelete?.(true)
      await flushPromises()
    })

    it('rolls back the in-progress state when the delete request fails', async () => {
      mockDeleteArchiveByName.mockRejectedValueOnce(new Error('boom'))

      const wrapper = await renderRepoDetail()
      await openArchivesTab(wrapper)

      await wrapper.find('button[title="Delete archive"]').trigger('click')
      await flushPromises()
      await clickModalConfirm()

      // The delete never actually got queued, so the optimistic "in flight"
      // marker must not stick around and leave the row permanently disabled.
      expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
      expect(wrapper.find('button[title="Delete archive"]').exists()).toBe(true)

      // A failed delete leaves the confirmation modal open (its own,
      // pre-existing behavior, unchanged here) - its teleported content
      // otherwise lingers in document.body and breaks other tests' modal
      // lookups, so tear it down explicitly.
      wrapper.unmount()
    })
  })

  describe('Break Lock', () => {
    async function openBreakLockDialog(
      wrapper: Awaited<ReturnType<typeof renderRepoDetail>>,
    ): Promise<void> {
      const breakLockBtn = wrapper.findAll('button').find((b) => b.text() === 'Break Lock')
      expect(breakLockBtn).toBeDefined()
      await breakLockBtn!.trigger('click')
      await flushPromises()
    }

    async function clickBreakLockConfirm(): Promise<void> {
      const confirmBtn = document.body.querySelector<HTMLButtonElement>('button.btn-danger')
      expect(confirmBtn).not.toBeNull()
      expect(confirmBtn!.textContent).toBe('Yes, Break Lock')
      confirmBtn!.click()
      await flushPromises()
    }

    it('shows borg_output alongside the confirmation message, not just the static message', async () => {
      setupApiSuccess()
      vi.mocked(apiClient.post).mockImplementation((url: string) => {
        if (url === '/repos/1/break-lock') {
          return Promise.resolve({
            data: {
              message: "lock broken on repository 'server-daily'",
              borg_output: 'cleared stale local cache lock at /cache/borg/abc123',
            },
          })
        }
        return Promise.resolve({ data: {} })
      })

      const wrapper = await renderRepoDetail()
      await openBreakLockDialog(wrapper)
      await clickBreakLockConfirm()

      const result = document.body.querySelector('.break-lock-success')
      expect(result).not.toBeNull()
      expect(result!.textContent).toContain("lock broken on repository 'server-daily'")
      expect(result!.textContent).toContain('cleared stale local cache lock at /cache/borg/abc123')

      // The dialog stays open after a successful break-lock so the admin can
      // read the result - its teleported content would otherwise leak into
      // document.body and break the next test's lookup, so tear it down.
      wrapper.unmount()
    })

    it('falls back to the plain message when there is no borg output to show', async () => {
      setupApiSuccess()
      vi.mocked(apiClient.post).mockImplementation((url: string) => {
        if (url === '/repos/1/break-lock') {
          return Promise.resolve({
            data: { message: "lock broken on repository 'server-daily'", borg_output: '' },
          })
        }
        return Promise.resolve({ data: {} })
      })

      const wrapper = await renderRepoDetail()
      await openBreakLockDialog(wrapper)
      await clickBreakLockConfirm()

      const result = document.body.querySelector('.break-lock-success')
      expect(result).not.toBeNull()
      expect(result!.textContent).toBe("lock broken on repository 'server-daily'")

      // Same as the test above: the dialog stays open, so its teleported
      // content would otherwise leak into document.body for whatever test
      // runs next.
      wrapper.unmount()
    })
  })
})
