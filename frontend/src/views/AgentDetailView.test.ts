// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises, type DOMWrapper, type VueWrapper } from '@vue/test-utils'
import { ref, nextTick, type ComponentPublicInstance } from 'vue'
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import AgentDetailView from './AgentDetailView.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'

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

// `status` is a real ref: the view watches it to refetch on reconnect, and
// leaving it undefined made Vue warn about an invalid watch source on every
// mount in this file.
const wsStatus = ref<'connected' | 'disconnected'>('connected')

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: (type: string, cb: (p: unknown) => void) => {
      wsHandlers[type] = cb
    },
    status: wsStatus,
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
  default_pre_backup_commands: [],
  default_post_backup_commands: [],
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
    error_message: 'some file changed during backup',
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

describe('AgentDetailView — backups tab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows filter buttons on the backups tab', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    expect(wrapper.text()).toMatch(/Newest|Oldest/)
  })

  it('renders all reports by default', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    expect(wrapper.findAll('.result-card')).toHaveLength(3)
  })

  it('shows the repo and schedule name on each report so a failure can be traced', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
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

  it('shows the Warnings box but not a duplicate Error box for a warning-only report', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const warningCard = wrapper
      .findAll('.result-card')
      .find((c) => c.classes().includes('result-warning'))
    expect(warningCard).toBeDefined()
    await warningCard!.trigger('click')

    expect(warningCard!.find('.result-warnings').exists()).toBe(true)
    expect(warningCard!.find('.result-error').exists()).toBe(false)
  })

  it('filters to only failed reports when Failed is clicked', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    expect(wrapper.text()).toContain('No backup reports available.')
  })

  it('highlights the report matching the archive query param', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups', archive: 'test-host-2026-06-02T10:00:00' } })
    await flushPromises()

    expect(wrapper.text()).toContain('some file changed during backup')
  })

  async function mountBackupsWithStatus(
    reports: unknown[],
    status: string,
  ): Promise<VueWrapper<ComponentPublicInstance>> {
    setupApi(reports)
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups', status } })
    await flushPromises()
    return wrapper
  }

  it('pins, expands and highlights the newest report matching the status query param', async () => {
    const wrapper = await mountBackupsWithStatus(mockReports, 'failed')

    const highlighted = wrapper.find('.result-card-highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.classes()).toContain('result-failed')
    expect(wrapper.text()).toContain('Connection refused')
  })

  it('pins the newest report when several share the status query param', async () => {
    const olderFailed = { ...mockReports[2], id: 4, finished_at: '2026-06-02T10:00:00Z' }
    const newerFailed = { ...mockReports[2], id: 5, finished_at: '2026-06-04T10:00:00Z' }
    const wrapper = await mountBackupsWithStatus([olderFailed, newerFailed], 'failed')

    const highlighted = wrapper.find('.result-card-highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.attributes('id')).toBe('report-5')
  })

  it('re-pins the matching report when the status query param changes on an already-mounted page', async () => {
    const wrapper = await mountBackupsWithStatus(mockReports, 'failed')

    let highlighted = wrapper.find('.result-card-highlighted')
    expect(highlighted.attributes('id')).toBe('report-3')

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups', status: 'warning' } })
    await flushPromises()

    highlighted = wrapper.find('.result-card-highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.attributes('id')).toBe('report-2')
  })

  it('clears the pinned highlight and auto-expand when the status query param is removed', async () => {
    const wrapper = await mountBackupsWithStatus(mockReports, 'failed')

    expect(wrapper.find('.result-card-highlighted').exists()).toBe(true)
    expect(wrapper.text()).toContain('Click to collapse')

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups' } })
    await flushPromises()

    expect(wrapper.find('.result-card-highlighted').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Click to collapse')
  })
})

describe('AgentDetailView — schedules tab', () => {
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
    setupApi(mockReports, [{ id: 10, name: 'shared-repo' }], schedules)
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const agentSchedules = (
      wrapper.vm as unknown as { agentSchedules: Array<{ id: number; name: string }> }
    ).agentSchedules
    expect(agentSchedules).toEqual([{ ...schedules[0] }])
  })

  async function mountSchedulesTab(
    schedules: unknown[],
  ): Promise<VueWrapper<ComponentPublicInstance>> {
    setupApi(mockReports, [{ id: 10, name: 'shared-repo' }], schedules)
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openSchedulesTab(wrapper)
    return wrapper
  }

  it('renders schedule cards on the schedules tab', async () => {
    const wrapper = await mountSchedulesTab([
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
    ])

    expect(wrapper.text()).toContain('Nightly Backup')
    expect(wrapper.find('.entity-status-pill').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Sequential')
  })

  it('shows empty state when no schedules target the agent', async () => {
    setupApi(mockReports, [{ id: 10, name: 'shared-repo' }], [])
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openSchedulesTab(wrapper)

    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.find('.empty-title').text()).toBe('No schedules yet')
  })

  it('opens the schedule when its card is selected', async () => {
    const wrapper = await mountSchedulesTab([
      {
        id: 42,
        repo_id: 10,
        name: 'Nightly Backup',
        target_hostnames: ['test-host'],
        schedule_type: 'backup',
        cron_expression: '0 2 * * *',
        enabled: true,
        next_run_at: null,
      },
    ])

    await wrapper.find('.schedule-card').trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toBe('/schedules/42')
  })

  it('shows a Disabled pill and tints the card for a disabled schedule', async () => {
    const wrapper = await mountSchedulesTab([
      {
        id: 1,
        repo_id: 10,
        name: 'Weekend Archive',
        target_hostnames: ['test-host'],
        schedule_type: 'backup',
        cron_expression: '0 3 * * 0',
        enabled: false,
        next_run_at: null,
      },
    ])

    expect(wrapper.find('.entity-status-pill').text()).toBe('Disabled')
    expect(wrapper.find('.schedule-card').classes()).toContain('schedule-card-notable')
  })

  function setupApiWithHealth(
    schedules: unknown[],
    health: unknown[],
    reports: unknown[] = [],
  ): void {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/agents/test-host/repos')
        return Promise.resolve({ data: [{ id: 10, name: 'shared-repo' }] })
      if (url === '/schedules') return Promise.resolve({ data: schedules })
      if (url === '/agents/test-host/reports') return Promise.resolve({ data: reports })
      if (url === '/stats/health') return Promise.resolve({ data: health })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
  }

  const overdueSchedule = [
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

  const overdueHealth = Object.freeze([
    {
      schedule_id: 1,
      hostname: 'test-host',
      target_name: 'shared-repo',
      last_status: 'success',
      last_backup_at: '2026-01-01T00:00:00Z',
      is_overdue: true,
      last_error_message: null,
      cron_expression: '0 2 * * *',
      schedule_enabled: true,
    },
  ])

  function renderWithHealth() {
    setupApiWithHealth(overdueSchedule, overdueHealth)
    return renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
  }

  it('shows an Overdue issue chip on a schedule with no recent run', async () => {
    const wrapper = renderWithHealth()
    await flushPromises()
    await openSchedulesTab(wrapper)

    const chip = wrapper.find('.entity-issue-chip.sev-warning')
    expect(chip.text()).toBe('Overdue')
    expect(wrapper.find('.schedule-card').classes()).not.toContain('schedule-card-highlighted')
  })

  it('highlights overdue schedule cards when the health=overdue query param is set', async () => {
    const wrapper = renderWithHealth()
    await flushPromises()

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'schedules', health: 'overdue' } })
    await flushPromises()

    expect(wrapper.find('.schedule-card').classes()).toContain('schedule-card-highlighted')
  })
})

describe('AgentDetailView — backup progress', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    for (const key of Object.keys(wsHandlers)) delete wsHandlers[key]
  })

  async function startBackupOnHost(
    archiveName: string | null = null,
    eventHostname: string = 'test-host',
  ) {
    setupApi([], [{ id: 10, name: 'server-daily' }])
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: eventHostname,
      target_name: 'server-daily',
      archive_name: archiveName,
      schedule_id: 1,
    })
    await nextTick()
    return wrapper
  }

  it('BackupStarted for this host shows the backup in progress card', async () => {
    const wrapper = await startBackupOnHost('server-daily-2026-07-06')

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
    expect(wrapper.text()).toContain('Backup in progress')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('BackupStarted shows the backup in progress card with archive name', async () => {
    const wrapper = await startBackupOnHost('server-daily-2026-07-06')

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
    expect(wrapper.text()).toContain('Backup in progress')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('BackupStarted for a different host does not show a progress card', async () => {
    const wrapper = await startBackupOnHost(null, 'other-host')

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupCompleted hides the progress card', async () => {
    const wrapper = await startBackupOnHost()
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
    const wrapper = await startBackupOnHost()

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
    setupApi([runningReport], [{ id: 10, name: 'server-daily' }])
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
  })
})

describe('AgentDetailView — default file change patterns', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows empty state when no default patterns are configured', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
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
    const wrapper = renderWithPlugins(AgentDetailView, {
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

describe('AgentDetailView — deploy/upgrade button permission gate', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  function setupApiWithUpgradeAvailable(): void {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/system/version')
        return Promise.resolve({ data: { agent_version: '2.0.0', server_commit_count: null } })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
  }

  it('shows the Upgrade button when the user has can_upgrade_agent', async () => {
    setupApiWithUpgradeAvailable()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: true } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Upgrade')
  })

  it('hides the Upgrade button without can_upgrade_agent permission', async () => {
    setupApiWithUpgradeAvailable()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: false } } },
    })
    await flushPromises()

    expect(wrapper.text()).not.toContain('Upgrade')
  })
})

describe('AgentDetailView - identity, token and merge', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  async function render(
    agentOverrides: Record<string, unknown> = {},
  ): Promise<VueWrapper<ComponentPublicInstance>> {
    const agent = { ...mockAgent, ...agentOverrides }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [agent] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: true } } },
    })
    await flushPromises()
    return wrapper
  }

  async function clickButton(
    wrapper: VueWrapper<ComponentPublicInstance>,
    label: string,
  ): Promise<void> {
    const match = wrapper.findAll('button').find((b) => b.text().trim() === label)
    if (!match) throw new Error(`no button labelled "${label}"`)
    await match.trigger('click')
    await flushPromises()
  }

  /**
   * Scoped to the open dialog: the page behind it has its own Add Pattern
   * button in the alias panel, which an unscoped search finds first.
   */
  async function clickDialogButton(
    wrapper: VueWrapper<ComponentPublicInstance>,
    label: string,
  ): Promise<void> {
    const match = wrapper.findAll('.modal-footer button').find((b) => b.text().trim() === label)
    if (!match) throw new Error(`no dialog button labelled "${label}"`)
    await match.trigger('click')
    await flushPromises()
  }

  // The token is shown once and never again, so it has to reach the dialog
  // and be gone from the page as soon as the dialog is closed.
  it('reveals a regenerated token once and clears it on Done', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { agent: { ...mockAgent }, token: 'tok_regenerated' },
    } as never)

    await clickButton(wrapper, 'Regenerate Token')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/test-host/regenerate-token')
    expect(wrapper.find('.token-text').text()).toBe('tok_regenerated')

    await clickButton(wrapper, 'Copy')
    await clickDialogButton(wrapper, 'Done')

    expect(wrapper.find('.token-text').exists()).toBe(false)
  })

  it('opens the dialog with the error when regenerating fails', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent offline'))

    await clickButton(wrapper, 'Regenerate Token')

    expect(wrapper.find('.token-text').exists()).toBe(false)
    expect(openModals(wrapper)).toHaveLength(1)
    // extractError is stubbed to a fixed string in this file's mocks.
    expect(wrapper.text()).toContain('Unknown error')
  })

  it('clears the token when the dialog is dismissed rather than closed', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { agent: { ...mockAgent }, token: 'tok_regenerated' },
    } as never)
    await clickButton(wrapper, 'Regenerate Token')

    await dismissModal(wrapper)

    expect(wrapper.find('.token-text').exists()).toBe(false)
  })

  // Renaming a host breaks every archive borg already wrote under the old
  // name, so the old name is offered as an alias rather than assumed.
  it('offers the old hostname as an alias after a rename and saves it', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.put).mockResolvedValue({
      data: { ...mockAgent, hostname: 'renamed-host' },
    } as never)
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} } as never)

    await clickButton(wrapper, 'Edit')
    await wrapper.find('input[placeholder="hostname"]').setValue('renamed-host')
    await clickButton(wrapper, 'Save')

    expect(wrapper.text()).toContain('Hostname changed from')
    expect(wrapper.text()).toContain('renamed-host')

    await clickDialogButton(wrapper, 'Add Pattern')

    expect(apiClient.post).toHaveBeenCalledWith('/agents/renamed-host/hostname-patterns', {
      pattern: 'test-host',
    })
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('writes no alias when the offer is declined', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.put).mockResolvedValue({
      data: { ...mockAgent, hostname: 'renamed-host' },
    } as never)

    await clickButton(wrapper, 'Edit')
    await wrapper.find('input[placeholder="hostname"]').setValue('renamed-host')
    await clickButton(wrapper, 'Save')
    await clickDialogButton(wrapper, 'No')

    expect(apiClient.post).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('does not offer an alias when only the display name changed', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.put).mockResolvedValue({ data: { ...mockAgent } } as never)

    await clickButton(wrapper, 'Edit')
    await wrapper.find('input[placeholder="Optional friendly name"]').setValue('Test Box')
    await clickButton(wrapper, 'Save')

    expect(wrapper.text()).not.toContain('Hostname changed from')
  })

  it('closes the merge dialog for an imported agent on cancel', async () => {
    const wrapper = await render({ is_imported: true })

    await clickButton(wrapper, 'Merge into...')
    const dialog = wrapper.findComponent(MergeAgentDialog)
    expect(dialog.exists()).toBe(true)

    dialog.vm.$emit('cancel')
    await flushPromises()

    expect(wrapper.findComponent(MergeAgentDialog).exists()).toBe(false)
  })
})

describe('AgentDetailView - tab bar and list controls', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  // The tabs are also reachable by ?tab=, which is how the rest of this file
  // drives them - so clicking one is the only thing that exercises the tab
  // bar's own binding.
  it('switches tabs by clicking the tab bar', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const backupsTab = wrapper.findAll('button.tab').find((t) => t.text().includes('Backups'))
    expect(backupsTab).toBeDefined()
    await backupsTab!.trigger('click')
    await flushPromises()

    expect(backupsTab!.classes()).toContain('active')
    expect(wrapper.findAll('button').some((b) => b.text().trim() === 'Warning')).toBe(true)
  })

  // Newest first is the default; the toggle is what an operator uses to walk
  // a failure back to when it started.
  it('reverses the backup list order from the sort toggle', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const reports = (): string[] =>
      wrapper.findAll('.result-card').map((r) => r.attributes('id') ?? '')
    const before = reports()
    expect(before.length).toBeGreaterThan(1)

    const sortBtn = wrapper.findAll('button').find((b) => /Newest|Oldest/.test(b.text()))
    expect(sortBtn).toBeDefined()
    await sortBtn!.trigger('click')
    await flushPromises()

    expect(reports()).toEqual([...before].reverse())
  })
})
