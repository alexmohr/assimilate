// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises, type DOMWrapper, type VueWrapper } from '@vue/test-utils'
import { ref, nextTick, type ComponentPublicInstance } from 'vue'
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

// Captured WebSocket message handlers - populated during component setup().
const wsHandlers: Record<string, (payload: unknown) => void> = {}

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: (type: string, cb: (p: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
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
  extractBlobError: async (_e: unknown, fallback?: string): Promise<string> =>
    fallback ?? 'Unknown error',
}))

vi.mock('../components/MergeAgentDialog.vue', () => ({
  default: {
    name: 'MergeAgentDialog',
    template: '<div />',
    props: ['source', 'allAgents'],
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

const mockAgent = {
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
    repo_name: 'server-daily',
    schedule_id: 100,
    schedule_name: 'Nightly Server Backup',
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
    repo_name: 'server-daily',
    schedule_id: 100,
    schedule_name: 'Nightly Server Backup',
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
    repo_name: 'server-daily',
    schedule_id: 100,
    schedule_name: 'Nightly Server Backup',
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

function setupApi(reports = mockReports, repos: unknown[] = [], schedules: unknown[] = []): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
    if (url === '/agents/test-host/repos') return Promise.resolve({ data: repos })
    if (url === '/schedules') return Promise.resolve({ data: schedules })
    if (url === '/agents/test-host/reports') return Promise.resolve({ data: reports })
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

async function openSchedulesTab(wrapper: VueWrapper<ComponentPublicInstance>): Promise<void> {
  const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
  await router.push({ query: { tab: 'schedules' } })
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

  it('shows the repo and schedule name on each report so a failure can be traced', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const card = wrapper.findAll('.result-card')[0]
    expect(card.text()).toContain('server-daily')
    expect(card.text()).toContain('Nightly Server Backup')
    const scheduleLink = card.find('a.result-schedule-link')
    expect(scheduleLink.exists()).toBe(true)
    expect(scheduleLink.attributes('href')).toBe('/schedules/100')
  })

  it('omits the schedule link when a report has no schedule_id', async () => {
    setupApi([{ ...mockReports[0], schedule_id: null, schedule_name: null }])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const card = wrapper.findAll('.result-card')[0]
    expect(card.find('a.result-schedule-link').exists()).toBe(false)
    expect(card.text()).toContain('server-daily')
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

describe('HostDetailView — schedules tab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows only schedules that explicitly target the agent', async () => {
    const schedules = [
      {
        id: 1,
        repo_id: 10,
        name: 'Test host schedule',
        target_hostnames: ['test-host'],
        schedule_type: 'backup',
        cron_expression: '0 2 * * *',
        enabled: true,
      },
      {
        id: 2,
        repo_id: 10,
        name: 'Other host schedule',
        target_hostnames: ['other-host'],
        schedule_type: 'backup',
        cron_expression: '0 3 * * *',
        enabled: true,
      },
    ]
    setupApi(mockReports, [{ id: 10, target_name: 'shared-repo' }], schedules)
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const agentSchedules = (
      wrapper.vm as unknown as { agentSchedules: Array<{ id: number; name: string }> }
    ).agentSchedules
    expect(agentSchedules).toEqual([{ ...schedules[0] }])
  })

  it('renders schedule cards on the schedules tab', async () => {
    const schedules = [
      {
        id: 1,
        repo_id: 10,
        name: 'Nightly Backup',
        target_hostnames: ['test-host'],
        schedule_type: 'backup',
        cron_expression: '0 2 * * *',
        enabled: true,
        next_run_at: null,
      },
    ]
    setupApi(mockReports, [{ id: 10, target_name: 'shared-repo' }], schedules)
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openSchedulesTab(wrapper)

    expect(wrapper.text()).toContain('Nightly Backup')
    expect(wrapper.text()).toContain('Enabled')
    expect(wrapper.text()).not.toContain('Sequential')
  })

  it('shows empty state when no schedules target the agent', async () => {
    setupApi(mockReports, [{ id: 10, target_name: 'shared-repo' }], [])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openSchedulesTab(wrapper)

    expect(wrapper.text()).toContain('No schedules for this agent.')
  })
})

describe('HostDetailView — backup progress', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    for (const key of Object.keys(wsHandlers)) delete wsHandlers[key]
  })

  it('BackupStarted for this host shows the backup in progress card', async () => {
    setupApi([], [{ id: 10, target_name: 'server-daily' }])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'test-host',
      target_name: 'server-daily',
      archive_name: 'server-daily-2026-07-06',
      schedule_id: 1,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
    expect(wrapper.text()).toContain('Backup in progress')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('BackupStarted for a different host does not show a progress card', async () => {
    setupApi([], [{ id: 10, target_name: 'server-daily' }])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'other-host',
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupCompleted hides the progress card', async () => {
    setupApi([], [{ id: 10, target_name: 'server-daily' }])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'test-host',
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()
    expect(wrapper.find('.live-log-card').exists()).toBe(true)

    wsHandlers['BackupCompleted']?.({
      hostname: 'test-host',
      target_name: 'server-daily',
      report: { id: 1 },
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupLog with archive_progress JSON updates the progress data', async () => {
    setupApi([], [{ id: 10, target_name: 'server-daily' }])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'test-host',
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    wsHandlers['BackupLog']?.({
      hostname: 'test-host',
      schedule_id: 1,
      repo_id: 10,
      line: JSON.stringify({
        type: 'archive_progress',
        nfiles: 1234,
        original_size: 1024 * 1024,
        path: '/home/alex/documents/report.pdf',
      }),
    })
    await nextTick()

    expect(wrapper.text()).toContain('1,234')
    expect(wrapper.text()).toContain('/home/alex/documents/report.pdf')
  })

  it('shows the progress card on load when a report is already running', async () => {
    const runningReport = {
      ...mockReports[0],
      id: 99,
      status: 'started',
      started_at: '2026-07-06T09:55:00Z',
    }
    setupApi([runningReport], [{ id: 10, target_name: 'server-daily' }])
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
  })
})

describe('HostDetailView — default file change patterns', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows empty state when no default patterns are configured', async () => {
    setupApi()
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('No default file change patterns configured.')
  })

  it('lists parsed patterns with their action', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents')
        return Promise.resolve({
          data: [
            {
              ...mockAgent,
              default_file_change_patterns_raw: '*/tmp/* ignore\n*/etc/config* fatal',
            },
          ],
        })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('*/tmp/*')
    expect(text).toContain('ignore')
    expect(text).toContain('*/etc/config*')
    expect(text).toContain('fatal')
  })

  it('saves edited default file change patterns', async () => {
    setupApi()
    vi.mocked(apiClient.put).mockResolvedValue({
      data: { ...mockAgent, default_file_change_patterns_raw: '*/var/log* ignore' },
    })
    const wrapper = renderWithPlugins(HostDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const findCard = (): DOMWrapper<Element> =>
      wrapper.findAll('.info-card').find((c) => c.text().includes('Default File Change Patterns'))!

    await findCard().find('button').trigger('click')
    await findCard()
      .findAll('button')
      .find((b) => b.text() === '+ Add pattern')!
      .trigger('click')
    await findCard().find('input[type="text"]').setValue('*/var/log*')
    await findCard().findAll('select').at(-1)!.setValue('ignore')

    const saveBtn = findCard()
      .findAll('button')
      .find((b) => b.text() === 'Save')!
    await saveBtn.trigger('click')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/test-host',
      expect.objectContaining({ default_file_change_patterns_raw: '*/var/log* ignore' }),
    )
  })
})
