// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises, type DOMWrapper, type VueWrapper } from '@vue/test-utils'
import { ref, nextTick, type ComponentPublicInstance } from 'vue'
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import AgentDetailView from './AgentDetailView.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import { logger } from '../utils/logger'

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

const mockToastSuccess = vi.fn()
const mockToastError = vi.fn()
vi.mock('../composables/useToast', () => ({
  useToast: (): {
    success: ReturnType<typeof vi.fn>
    error: ReturnType<typeof vi.fn>
  } => ({
    success: mockToastSuccess,
    error: mockToastError,
  }),
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
    props: ['hostname', 'agentVersion', 'availableVersion', 'lastSshUser', 'forceRedeploy'],
  },
}))

import { apiClient } from '../api/client'
import { useEscapeKey } from '../composables/useEscapeKey'

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
    if (url === '/agents/test-host/reports/failed/count') {
      const count = reports.filter((r) => r.status === 'failed').length
      return Promise.resolve({ data: { count } })
    }
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

    const labels = wrapper.findAll('.segmented-option').map((b) => b.text())
    expect(labels).toEqual(['All 3', 'Success 1', 'Warning 1', 'Failed 1'])
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

    expect(wrapper.findAll('[id^="report-"]')).toHaveLength(3)
  })

  it('shows the repo and schedule name on each report so a failure can be traced', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const card = wrapper.findAll('[id^="report-"]')[0]
    expect(card.text()).toContain('server-daily')
    expect(card.text()).toContain('Nightly Server Backup')
    const scheduleLink = card.find('a.row-schedule-link')
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

    const card = wrapper.findAll('[id^="report-"]')[0]
    expect(card.find('a.row-schedule-link').exists()).toBe(false)
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

    const warningBtn = wrapper.findAll('button').find((b) => b.text().startsWith('Warning'))
    await warningBtn!.trigger('click')

    const rows = wrapper.findAll('[id^="report-"]')
    expect(rows).toHaveLength(1)
    expect(rows[0].find('.agent-row-stripe--warning').exists()).toBe(true)
  })

  it('shows the Warnings box but not a duplicate Error box for a warning-only report', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const warningRow = wrapper
      .findAll('[id^="report-"]')
      .find((c) => c.find('.agent-row-stripe--warning').exists())
    expect(warningRow).toBeDefined()
    await warningRow!.find('button[aria-expanded]').trigger('click')

    expect(wrapper.find('.group-label--warning').exists()).toBe(true)
    expect(wrapper.find('.group-label--danger').exists()).toBe(false)
  })

  it('filters to only failed reports when Failed is clicked', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openBackupsTab(wrapper)

    const failedBtn = wrapper.findAll('button').find((b) => b.text().startsWith('Failed'))
    await failedBtn!.trigger('click')

    const rows = wrapper.findAll('[id^="report-"]')
    expect(rows).toHaveLength(1)
    expect(rows[0].find('.agent-row-stripe--danger').exists()).toBe(true)
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
      .find((b) => b.text().startsWith('Warning'))!
      .trigger('click')
    expect(wrapper.findAll('[id^="report-"]')).toHaveLength(1)

    await wrapper
      .findAll('button')
      .find((b) => b.text().startsWith('All'))!
      .trigger('click')
    expect(wrapper.findAll('[id^="report-"]')).toHaveLength(3)
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
      .find((b) => b.text().startsWith('Failed'))!
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

    const highlighted = wrapper.find('.agent-row--highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.find('.agent-row-stripe--warning').exists()).toBe(true)
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

    const highlighted = wrapper.find('.agent-row--highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.find('.agent-row-stripe--danger').exists()).toBe(true)
    expect(wrapper.text()).toContain('Connection refused')
  })

  it('pins the newest report when several share the status query param', async () => {
    const olderFailed = { ...mockReports[2], id: 4, finished_at: '2026-06-02T10:00:00Z' }
    const newerFailed = { ...mockReports[2], id: 5, finished_at: '2026-06-04T10:00:00Z' }
    const wrapper = await mountBackupsWithStatus([olderFailed, newerFailed], 'failed')

    const highlighted = wrapper.find('.agent-row--highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.attributes('id')).toBe('report-5')
  })

  it('re-pins the matching report when the status query param changes on an already-mounted page', async () => {
    const wrapper = await mountBackupsWithStatus(mockReports, 'failed')

    let highlighted = wrapper.find('.agent-row--highlighted')
    expect(highlighted.attributes('id')).toBe('report-3')

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups', status: 'warning' } })
    await flushPromises()

    highlighted = wrapper.find('.agent-row--highlighted')
    expect(highlighted.exists()).toBe(true)
    expect(highlighted.attributes('id')).toBe('report-2')
  })

  it('clears the pinned highlight and auto-expand when the status query param is removed', async () => {
    const wrapper = await mountBackupsWithStatus(mockReports, 'failed')

    expect(wrapper.find('.agent-row--highlighted').exists()).toBe(true)
    expect(wrapper.text()).toContain('Hide detail')

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'backups' } })
    await flushPromises()

    expect(wrapper.find('.agent-row--highlighted').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Hide detail')
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

  it('renders schedule rows on the schedules tab', async () => {
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
    expect(wrapper.findAll('.rows .agent-row')).toHaveLength(1)
    expect(wrapper.text()).not.toContain('Disabled')
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

  it('opens the schedule when its name is clicked', async () => {
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

    await wrapper.find('.agent-row-name').trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toBe('/schedules/42')
  })

  it('shows a Disabled badge and a muted stripe for a disabled schedule', async () => {
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
    expect(wrapper.find('.agent-row-stripe--muted').exists()).toBe(true)
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
    expect(wrapper.find('.rows .agent-row').classes()).not.toContain('agent-row--highlighted')
  })

  // The chips are links into the filtered activity log, not decoration. An
  // earlier revision of the row rendered plain badges and silently dropped
  // that; only the e2e suite noticed.
  it('navigates to the filtered activity log from a Failed chip', async () => {
    setupApiWithHealth(overdueSchedule, [
      { ...overdueHealth[0], is_overdue: false, last_status: 'failed' },
    ])
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openSchedulesTab(wrapper)

    const chip = wrapper.find('.entity-issue-chip.sev-danger')
    expect(chip.text()).toBe('Failed')
    await chip.trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toBe(
      '/activity?category=backup&schedule_id=1&status=failed',
    )
  })

  it('highlights overdue schedule rows when the health=overdue query param is set', async () => {
    const wrapper = renderWithHealth()
    await flushPromises()

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { tab: 'schedules', health: 'overdue' } })
    await flushPromises()

    expect(wrapper.find('.rows .agent-row').classes()).toContain('agent-row--highlighted')
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
      started_at: new Date().toISOString(),
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

  // Regression test: a page reload reconnects the WS, and the server replays
  // a BackupStarted catch-up event for every already-running backup so the
  // reloaded page knows about it. That event must carry the backup's real
  // start time - if the frontend defaulted to Date.now() instead, the
  // elapsed clock would reset to ~0 on every reload instead of showing how
  // long the backup has actually been running.
  it('seeds elapsed time from the real backup start carried on BackupStarted, not the reload time', async () => {
    setupApi([], [{ id: 10, name: 'server-daily' }])
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const tenMinutesAgo = new Date(Date.now() - 10 * 60_000).toISOString()
    wsHandlers['BackupStarted']?.({
      hostname: 'test-host',
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
      started_at: tenMinutesAgo,
    })
    // The card shows "Waiting for progress..." until progress data arrives.
    wsHandlers['BackupLog']?.({
      hostname: 'test-host',
      schedule_id: 1,
      repo_id: 10,
      line: JSON.stringify({ type: 'archive_progress', nfiles: 1, original_size: 1, path: 'x' }),
    })
    await nextTick()

    const elapsedRow = wrapper
      .findAll('.live-stat-row')
      .find((r) => r.find('.live-stat-label').text() === 'Elapsed')
    expect(elapsedRow).toBeDefined()
    // ~10 minutes elapsed, not "0s" - proves it used started_at, not Date.now().
    expect(elapsedRow!.find('.live-stat-value').text()).toMatch(/^(9|10)m/)
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

  it('cancels a running backup and shows a success toast', async () => {
    const wrapper = await startBackupOnHost()
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} } as never)

    const cancelBtn = wrapper.find('.live-log-header-actions button')
    expect(cancelBtn.text()).toBe('Cancel backup')
    await cancelBtn.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/test-host/repos/10/cancel-backup',
      {},
      { params: {} },
    )
    expect(mockToastSuccess).toHaveBeenCalledWith('Cancel request sent.')
  })

  it('shows an error toast and re-enables the button when cancelling fails', async () => {
    const wrapper = await startBackupOnHost()
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent offline'))

    const cancelBtn = wrapper.find('.live-log-header-actions button')
    await cancelBtn.trigger('click')
    await flushPromises()

    expect(mockToastError).toHaveBeenCalledWith('Unknown error')
    expect(wrapper.find('.live-log-header-actions button').attributes('disabled')).toBeUndefined()
  })
})

describe('AgentDetailView — power phase badge', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    for (const key of Object.keys(wsHandlers)) delete wsHandlers[key]
  })

  async function mountAgent(): Promise<VueWrapper<ComponentPublicInstance>> {
    setupApi([], [{ id: 10, name: 'server-daily' }])
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    return wrapper
  }

  it('shows the phase label from a source-target RunEvent for this host', async () => {
    const wrapper = await mountAgent()

    wsHandlers['RunEvent']?.({
      run_id: 'run-1',
      target: 'source',
      event_type: 'wake_sent',
      message: 'Sent Wake-on-LAN packet to 3C:97:0E:2B:9A:44',
      occurred_at: '2026-06-01T03:00:00Z',
      hostname: 'test-host',
    })
    await nextTick()

    expect(wrapper.find('.badge--info').text()).toContain('Waking host...')
  })

  it('ignores a RunEvent for a different agent', async () => {
    const wrapper = await mountAgent()

    wsHandlers['RunEvent']?.({
      run_id: 'run-1',
      target: 'source',
      event_type: 'wake_sent',
      message: 'Sent Wake-on-LAN packet',
      occurred_at: '2026-06-01T03:00:00Z',
      hostname: 'other-host',
    })
    await nextTick()

    expect(wrapper.text()).not.toContain('Waking host...')
  })

  it('ignores a repository-target RunEvent for this same run', async () => {
    const wrapper = await mountAgent()

    wsHandlers['RunEvent']?.({
      run_id: 'run-1',
      target: 'repository',
      event_type: 'wake_sent',
      message: 'Sent Wake-on-LAN packet',
      occurred_at: '2026-06-01T03:00:00Z',
      hostname: 'test-host',
    })
    await nextTick()

    expect(wrapper.text()).not.toContain('Waking host...')
  })

  it('clears the phase and shows Online once the agent actually connects', async () => {
    const wrapper = await mountAgent()

    wsHandlers['RunEvent']?.({
      run_id: 'run-1',
      target: 'source',
      event_type: 'wake_sent',
      message: 'Sent Wake-on-LAN packet',
      occurred_at: '2026-06-01T03:00:00Z',
      hostname: 'test-host',
    })
    await nextTick()
    expect(wrapper.find('.badge--info').exists()).toBe(true)

    wsHandlers['AgentConnected']?.({ hostname: 'test-host' })
    await nextTick()

    expect(wrapper.find('.badge--info').exists()).toBe(false)
  })

  // An unrelated agent connecting elsewhere while this page is open must not
  // clear a phase that belongs to the agent actually shown on this page.
  // Note: this only covers the different-hostname case. `AgentConnected`
  // carries no `domain`, so two agents sharing the same hostname across
  // different domains can still cross-contaminate each other's phase here -
  // a known, low-impact limitation (self-heals via the safety-net timeout
  // below) that would need `domain`/`agent_id` added to the WS payload to
  // close, not something this test can exercise as written.
  it('does not clear the phase when a different agent connects', async () => {
    const wrapper = await mountAgent()

    wsHandlers['RunEvent']?.({
      run_id: 'run-1',
      target: 'source',
      event_type: 'wake_sent',
      message: 'Sent Wake-on-LAN packet',
      occurred_at: '2026-06-01T03:00:00Z',
      hostname: 'test-host',
    })
    await nextTick()
    expect(wrapper.find('.badge--info').exists()).toBe(true)

    wsHandlers['AgentConnected']?.({ hostname: 'some-other-host' })
    await nextTick()

    expect(wrapper.find('.badge--info').exists()).toBe(true)
  })

  // If the SSH command behind a shutdown/agent-stop attempt fails partway,
  // the agent never disconnects and never reconnects, so neither a
  // `host_offline` RunEvent nor an `AgentConnected` message ever arrives to
  // clear the phase - it would otherwise stay stuck forever.
  it('clears a stuck phase on its own after the safety-net timeout', async () => {
    vi.useFakeTimers()
    try {
      const wrapper = await mountAgent()

      wsHandlers['RunEvent']?.({
        run_id: 'run-1',
        target: 'source',
        event_type: 'shutdown_sent',
        message: 'Shutting down host',
        occurred_at: '2026-06-01T03:00:00Z',
        hostname: 'test-host',
      })
      await nextTick()
      expect(wrapper.text()).toContain('Shutting down...')

      await vi.advanceTimersByTimeAsync(5 * 60 * 1000)

      expect(wrapper.text()).not.toContain('Shutting down...')
    } finally {
      vi.useRealTimers()
    }
  })
})

async function openDefaultsSettings(wrapper: VueWrapper<ComponentPublicInstance>): Promise<void> {
  const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
  await router.push({ query: { tab: 'settings', section: 'defaults' } })
  await flushPromises()
}

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
    await openDefaultsSettings(wrapper)

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
    await openDefaultsSettings(wrapper)

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
    await openDefaultsSettings(wrapper)

    const findCard = (): DOMWrapper<Element> => wrapper.find('.settings-pane')

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
      { params: {} },
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

  function setupApiAlreadyCurrent(): void {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/system/version')
        return Promise.resolve({ data: { agent_version: '1.0.0', server_commit_count: null } })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
  }

  // Once the agent is already current there is nothing to upgrade, but the
  // host may still need reinstalling (a reimaged machine, a lost service
  // unit) - that is Redeploy, reached through the menu rather than the
  // primary slot.
  it('offers Redeploy agent from the menu once the agent is already current', async () => {
    setupApiAlreadyCurrent()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: true } } },
    })
    await flushPromises()
    expect(wrapper.text()).not.toContain('Upgrade')

    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Redeploy agent')!
      .trigger('click')
    await flushPromises()

    const dialog = wrapper.findComponent({ name: 'AgentDeployDialog' })
    expect(dialog.exists()).toBe(true)
    expect(dialog.props('forceRedeploy')).toBe(true)
  })

  it('hides Redeploy agent from the menu without can_upgrade_agent permission', async () => {
    setupApiAlreadyCurrent()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: false } } },
    })
    await flushPromises()

    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    expect(
      wrapper.findAll('.overflow-menu-item').some((i) => i.text().trim() === 'Redeploy agent'),
    ).toBe(false)
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

  /** Opens the header overflow menu, where the rare actions now live. */
  async function clickMenuAction(
    wrapper: VueWrapper<ComponentPublicInstance>,
    label: string,
  ): Promise<void> {
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    const match = wrapper.findAll('.overflow-menu-item').find((b) => b.text().trim() === label)
    if (!match) throw new Error(`no menu item labelled "${label}"`)
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

    await clickMenuAction(wrapper, 'Regenerate token')

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/test-host/regenerate-token',
      {},
      { params: {} },
    )
    expect(wrapper.find('.token-text').text()).toBe('tok_regenerated')

    await clickButton(wrapper, 'Copy')
    await clickDialogButton(wrapper, 'Done')

    expect(wrapper.find('.token-text').exists()).toBe(false)
  })

  it('opens the dialog with the error when regenerating fails', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent offline'))

    await clickMenuAction(wrapper, 'Regenerate token')

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
    await clickMenuAction(wrapper, 'Regenerate token')

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

    await clickMenuAction(wrapper, 'Edit identity')
    await wrapper.find('input[placeholder="hostname"]').setValue('renamed-host')
    await clickButton(wrapper, 'Save')

    expect(wrapper.text()).toContain('Hostname changed from')
    expect(wrapper.text()).toContain('renamed-host')

    await clickDialogButton(wrapper, 'Add pattern')

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/renamed-host/hostname-patterns',
      { pattern: 'test-host' },
      { params: {} },
    )
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('writes no alias when the offer is declined', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.put).mockResolvedValue({
      data: { ...mockAgent, hostname: 'renamed-host' },
    } as never)

    await clickMenuAction(wrapper, 'Edit identity')
    await wrapper.find('input[placeholder="hostname"]').setValue('renamed-host')
    await clickButton(wrapper, 'Save')
    await clickDialogButton(wrapper, 'No')

    expect(apiClient.post).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('does not offer an alias when only the display name changed', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.put).mockResolvedValue({ data: { ...mockAgent } } as never)

    await clickMenuAction(wrapper, 'Edit identity')
    await wrapper.find('input[placeholder="Optional friendly name"]').setValue('Test Box')
    await clickButton(wrapper, 'Save')

    expect(wrapper.text()).not.toContain('Hostname changed from')
  })

  // Changing only the domain must not trigger the hostname-rename alias
  // offer, but it does need to land in the URL so a bookmarked/shared link
  // to this agent keeps resolving to it once its domain is set.
  it('updates the URL domain query when only the domain changed', async () => {
    const wrapper = await render({ domain: null })
    vi.mocked(apiClient.put).mockResolvedValue({
      data: { ...mockAgent, domain: 'lab.example.com' },
    } as never)

    await clickMenuAction(wrapper, 'Edit identity')
    await wrapper.find('#identity-domain').setValue('lab.example.com')
    await clickButton(wrapper, 'Save')

    expect(wrapper.text()).not.toContain('Hostname changed from')
    const route = (wrapper.vm as unknown as { $route: { query: Record<string, unknown> } }).$route
    expect(route.query.domain).toBe('lab.example.com')
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

  // Regression test: MergeAgentDialog operates on the currently-resolved
  // `agent`. If a background refresh discovers a second agent now sharing
  // this hostname while the dialog is open, it must close rather than keep
  // running against the now-ambiguous/stale agent.
  it('closes the merge dialog if a background refresh makes the host ambiguous', async () => {
    const wrapper = await render({ is_imported: true })
    await clickButton(wrapper, 'Merge into...')
    expect(wrapper.findComponent(MergeAgentDialog).exists()).toBe(true)

    const otherAgent = { ...mockAgent, id: 2, domain: 'b.example.com' }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents')
        return Promise.resolve({ data: [{ ...mockAgent, is_imported: true }, otherAgent] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(wrapper.text()).toContain('More than one host is named')
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
    expect(wrapper.findAll('.segmented-option').some((b) => b.text().startsWith('Warning'))).toBe(
      true,
    )
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
      wrapper.findAll('[id^="report-"]').map((r) => r.attributes('id') ?? '')
    const before = reports()
    expect(before.length).toBeGreaterThan(1)

    const sortBtn = wrapper.findAll('button').find((b) => /Newest|Oldest/.test(b.text()))
    expect(sortBtn).toBeDefined()
    await sortBtn!.trigger('click')
    await flushPromises()

    expect(reports()).toEqual([...before].reverse())
  })
})

describe('AgentDetailView - tab structure and settings', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

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

  async function render(
    agentOverrides: Record<string, unknown> = {},
  ): Promise<VueWrapper<ComponentPublicInstance>> {
    const agent = { ...mockAgent, ...agentOverrides }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [agent] })
      if (url === '/agents/test-host/repos')
        return Promise.resolve({ data: [{ id: 10, name: 'shared-repo' }] })
      if (url === '/schedules') return Promise.resolve({ data: schedules })
      if (url === '/agents/test-host/reports') return Promise.resolve({ data: mockReports })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    return wrapper
  }

  function tabLabels(wrapper: VueWrapper<ComponentPublicInstance>): string[] {
    return wrapper.findAll('button.tab').map((t) => t.text().replace(/\s+/g, ' ').trim())
  }

  async function goTo(
    wrapper: VueWrapper<ComponentPublicInstance>,
    query: Record<string, string>,
  ): Promise<void> {
    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query })
    await flushPromises()
  }

  // Settings is a fourth tab rather than a header link: it costs no new route,
  // it matches the ?tab= convention the other detail views use, and it keeps
  // the header's action row at navigation plus an overflow.
  it('offers settings as a fourth tab', async () => {
    const wrapper = await render()
    expect(tabLabels(wrapper)).toEqual(['Overview', 'Schedules 1', 'Backups 3', 'Settings'])
  })

  it('opens the settings tab in place, without leaving the route', async () => {
    const wrapper = await render()
    const settingsTab = wrapper.findAll('button.tab').find((t) => t.text().includes('Settings'))
    await settingsTab!.trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { path: string } } } })
      .$router
    expect(router.currentRoute.value.path).toBe('/')
    expect(wrapper.find('.settings-nav').exists()).toBe(true)
  })

  it('deep-links to a settings section', async () => {
    const wrapper = await render()
    await goTo(wrapper, { tab: 'settings', section: 'aliases' })

    const current = wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.attributes('aria-current') === 'true')
    expect(current!.text()).toBe('Hostname aliases')
  })

  it('records the chosen settings section in the URL', async () => {
    const wrapper = await render()
    await goTo(wrapper, { tab: 'settings' })

    await wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.text() === 'Danger zone')!
      .trigger('click')
    await flushPromises()

    const router = (
      wrapper.vm as { $router: { currentRoute: { value: { query: Record<string, string> } } } }
    ).$router
    expect(router.currentRoute.value.query.section).toBe('danger')
  })

  // Configuration used to stack up under the agent's status on the landing
  // tab, so the build timestamp got the same billing as an overdue backup.
  it('keeps configuration off the landing tab', async () => {
    const wrapper = await render()
    expect(wrapper.text()).not.toContain('Backup defaults')
    expect(wrapper.text()).not.toContain('Danger zone')

    await goTo(wrapper, { tab: 'settings', section: 'defaults' })
    expect(wrapper.text()).toContain('Backup defaults')
  })

  // An empty tab that explains itself beats a tab bar whose contents shift
  // depending on which host was opened.
  it('keeps every tab for an imported host', async () => {
    const wrapper = await render({ is_imported: true })
    expect(tabLabels(wrapper)).toEqual(['Overview', 'Schedules 1', 'Backups 3', 'Settings'])
  })

  it('offers no agent-only settings for an imported host', async () => {
    const wrapper = await render({ is_imported: true })
    await goTo(wrapper, { tab: 'settings', section: 'identity' })

    expect(wrapper.text()).not.toContain('Connection')
    expect(wrapper.text()).not.toContain('Agent version')
  })

  it('hides the danger zone from a non-admin', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'viewer' } } },
    })
    await flushPromises()
    await goTo(wrapper, { tab: 'settings' })

    expect(wrapper.findAll('.settings-nav-item').map((b) => b.text())).toEqual([
      'Identity',
      'Backup defaults',
      'Hostname aliases',
    ])
  })

  // The inline panel appeared mid-page and pushed six cards down; every other
  // form in the app opens through BaseModal.
  it('edits identity in a dialog rather than an inline panel', async () => {
    const wrapper = await render()
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Edit identity')!
      .trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
    expect(wrapper.find('input[placeholder="hostname"]').exists()).toBe(true)
  })

  // A successful run is a link to what it produced.
  it('opens the archive list for a successful backup', async () => {
    const wrapper = await render()
    await goTo(wrapper, { tab: 'backups' })

    const successRow = wrapper
      .findAll('[id^="report-"]')
      .find((r) => r.find('button.agent-row-name').exists())
    await successRow!.find('button.agent-row-name').trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toContain('/repos/10')
    expect(router.currentRoute.value.fullPath).toContain('tab=archives')
    expect(router.currentRoute.value.fullPath).toContain('archive=')
  })

  // The link only appears when the preview is hiding something, so this needs
  // more runs than the preview shows.
  it('follows the Overview preview through to the full tab', async () => {
    const manyReports = Array.from({ length: 8 }, (_, i) => ({
      ...mockReports[0],
      id: 100 + i,
      finished_at: `2026-06-0${(i % 8) + 1}T10:00:00Z`,
    }))
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/agents/test-host/reports') return Promise.resolve({ data: manyReports })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    const backupsLink = wrapper
      .findAll('.section-link')
      .find((l) => l.text().includes('View all 8'))
    expect(backupsLink).toBeDefined()
    await backupsLink!.trigger('click')
    await flushPromises()

    const router = (
      wrapper.vm as { $router: { currentRoute: { value: { query: Record<string, string> } } } }
    ).$router
    expect(router.currentRoute.value.query.tab).toBe('backups')
  })

  it('opens the deploy dialog from the header', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/system/version')
        return Promise.resolve({ data: { agent_version: '2.0.0', server_commit_count: null } })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: true } } },
    })
    await flushPromises()

    await wrapper
      .findAll('.detail-actions > button')
      .find((b) => b.text().includes('Upgrade'))!
      .trigger('click')
    await flushPromises()

    const dialog = wrapper.findComponent({ name: 'AgentDeployDialog' })
    expect(dialog.exists()).toBe(true)
    expect(dialog.props('availableVersion')).toBe('2.0.0')
  })

  it('closes the deploy dialog when it is dismissed', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/system/version')
        return Promise.resolve({ data: { agent_version: '2.0.0', server_commit_count: null } })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin', can_upgrade_agent: true } } },
    })
    await flushPromises()

    await wrapper
      .findAll('.detail-actions > button')
      .find((b) => b.text().includes('Upgrade'))!
      .trigger('click')
    await flushPromises()

    wrapper.findComponent({ name: 'AgentDeployDialog' }).vm.$emit('close')
    await flushPromises()

    expect(wrapper.findComponent({ name: 'AgentDeployDialog' }).exists()).toBe(false)
  })

  async function openSshKeyDialog(wrapper: VueWrapper<ComponentPublicInstance>): Promise<void> {
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Deploy SSH key')!
      .trigger('click')
    await flushPromises()
    expect(openModals(wrapper)).toHaveLength(1)
  }

  it('closes the SSH key dialog from its footer', async () => {
    const wrapper = await render()
    await openSshKeyDialog(wrapper)

    await wrapper
      .findAll('.modal-footer button')
      .find((b) => b.text().trim() === 'Close')!
      .trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(0)
  })

  // Dismissing (backdrop, the dialog's own close) is wired separately from the
  // footer button, so a dialog can close cleanly one way and stick the other.
  it('closes the SSH key dialog when it is dismissed', async () => {
    const wrapper = await render()
    await openSshKeyDialog(wrapper)

    await dismissModal(wrapper)

    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('closes the SSH key dialog on Escape', async () => {
    const wrapper = await render()
    await openSshKeyDialog(wrapper)

    // useEscapeKey is mocked in this file, so the registered callback has to be
    // invoked directly. Only the dialog that is actually open has a true ref.
    const registration = vi
      .mocked(useEscapeKey)
      .mock.calls.find(([active]) => (active as { value: boolean }).value === true)
    expect(registration).toBeDefined()
    registration![1]()
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(0)
  })

  // Regression test: the SSH key dialog is gated `v-if="agent"` and hands
  // `agent.hostname` straight to the panel inside it. If this host drops out
  // of the list entirely during a background refresh (renamed/merged/deleted
  // from another session), `agent` is deliberately left at its last-good
  // value so the page behind the dialog doesn't blank - but that must not
  // leave the dialog itself open and acting on the now-orphaned agent.
  it('closes the SSH key dialog if a background refresh can no longer find the host', async () => {
    const wrapper = await render()
    await openSshKeyDialog(wrapper)

    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('deploys an SSH key from a dialog', async () => {
    const wrapper = await render()
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Deploy SSH key')!
      .trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
  })
})

describe('AgentDetailView - adoption, restart and live updates', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    for (const key of Object.keys(wsHandlers)) delete wsHandlers[key]
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
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    return wrapper
  }

  async function clickAction(
    wrapper: VueWrapper<ComponentPublicInstance>,
    label: string,
  ): Promise<void> {
    const match = wrapper.findAll('.detail-actions > button').find((b) => b.text().trim() === label)
    if (!match) throw new Error(`no action labelled "${label}"`)
    await match.trigger('click')
    await flushPromises()
  }

  // Adopting drops the "(imported)" suffix the import gave the host, then
  // issues it a real token - the host stops being a placeholder.
  it('adopts an imported host and reveals its new token', async () => {
    const wrapper = await render({ is_imported: true, display_name: 'old-web (imported)' })
    vi.mocked(apiClient.put).mockResolvedValue({ data: {} } as never)
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { agent: { ...mockAgent, id: '1' }, token: 'tok_adopted' },
    } as never)

    await clickAction(wrapper, 'Adopt')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/test-host',
      { display_name: 'old-web', domain: undefined },
      { params: {} },
    )
    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/test-host/regenerate-token',
      {},
      { params: {} },
    )
    expect(wrapper.find('.token-text').text()).toBe('tok_adopted')
  })

  it('keeps the page usable when adoption fails', async () => {
    const wrapper = await render({ is_imported: true })
    vi.mocked(apiClient.put).mockRejectedValue(new Error('already adopted'))

    await clickAction(wrapper, 'Adopt')

    expect(wrapper.find('.token-text').exists()).toBe(false)
    expect(wrapper.find('.detail-name').exists()).toBe(true)
  })

  it('leaves the agent list after a merge', async () => {
    const wrapper = await render({ is_imported: true })
    await clickAction(wrapper, 'Merge into...')

    wrapper.findComponent(MergeAgentDialog).vm.$emit('merged')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { path: string } } } })
      .$router
    expect(router.currentRoute.value.path).toBe('/agents')
  })

  it('restarts the agent from the overflow menu', async () => {
    const wrapper = await render({ supports_restart: true })
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} } as never)

    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Restart agent')!
      .trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/test-host/restart', {}, { params: {} })
  })

  it('reports a restart that could not be delivered', async () => {
    const wrapper = await render({ supports_restart: true })
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent offline'))

    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Restart agent')!
      .trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
  })

  it('navigates to this agent’s activity log from the overflow menu', async () => {
    const wrapper = await render()
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text().trim() === 'Activity log')!
      .trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toBe('/activity?category=backup&hostname=test-host')
  })

  it('reports an agent that is not in the list', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    // extractError is stubbed to a fixed string in this file, so the thrown
    // "not found" message cannot be asserted - only that it surfaced at all.
    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(wrapper.find('.detail-name').exists()).toBe(false)
  })

  // The page is long-lived, so it refetches on anything that could have
  // changed what it is showing rather than waiting for a navigation.
  it.each([
    ['DataChanged', {}],
    ['AgentConnected', { hostname: 'test-host' }],
    ['AgentDisconnected', {}],
  ])('refetches on %s', async (event, payload) => {
    await render()
    const before = vi.mocked(apiClient.get).mock.calls.length

    wsHandlers[event]?.(payload)
    await flushPromises()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBeGreaterThan(before)
  })

  it('refetches when the socket reconnects', async () => {
    await render()
    const before = vi.mocked(apiClient.get).mock.calls.length

    wsStatus.value = 'disconnected'
    await flushPromises()
    wsStatus.value = 'connected'
    await flushPromises()

    expect(vi.mocked(apiClient.get).mock.calls.length).toBeGreaterThan(before)
  })

  // Regression test: a background refetch used to run through the same
  // loading flag as the initial load, which unmounted and remounted the
  // whole page - discarding any edit in progress in a settings form. A
  // background refresh must update the page in place instead.
  it('does not show the loading spinner or remount the page on a background refetch', async () => {
    const wrapper = await render()
    const detailNameEl = wrapper.find('.detail-name').element

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(wrapper.find('.spinner-wrapper').exists()).toBe(false)
    expect(wrapper.find('.detail-name').element).toBe(detailNameEl)
  })

  it('logs and keeps showing the page when a background refetch fails', async () => {
    const wrapper = await render()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.reject(new Error('network error'))
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(logger.error).toHaveBeenCalledWith('background agent refresh failed', expect.any(Error))
    expect(wrapper.find('.error-banner').exists()).toBe(false)
    expect(wrapper.find('.detail-name').exists()).toBe(true)
  })

  // Regression test: fetchAgent used to null out `agent.value` as soon as
  // the current host dropped out of the list, then throw - so a background
  // refresh where the request itself *succeeds* but this host is no longer
  // among the results (deleted, or renamed/merged away from another
  // session) blanked the whole page instead of leaving the last-good data
  // on screen the way a failed request does.
  it('keeps showing the last-good agent when a background refetch no longer finds it', async () => {
    const wrapper = await render()
    expect(wrapper.find('.detail-name').exists()).toBe(true)

    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(logger.error).toHaveBeenCalledWith('background agent refresh failed', expect.any(Error))
    expect(wrapper.find('.error-banner').exists()).toBe(false)
    expect(wrapper.find('.detail-name').exists()).toBe(true)
  })

  // Regression test: unlike a background refresh, a genuine navigation to a
  // hostname that doesn't resolve goes through loadAgent()'s spinner/error
  // path, not refreshAgent()'s "preserve last-good state" one - so it must
  // still clear stale data. The agent-detail route has no `:key`, so the
  // same component instance (and its `agent` ref) is reused across a
  // hostname change; without resetting `agent.value` up front, the
  // breadcrumb's domain hint (which reads `agent.value` directly) would
  // keep showing the previous host's domain next to the new, nonexistent
  // one, right beside the "not found" error banner.
  it('clears the stale breadcrumb domain hint when navigating to a host that is not found', async () => {
    const wrapper = await render({ domain: 'a.example.com' })
    expect(wrapper.find('.detail-breadcrumb .muted').text()).toBe('(a.example.com)')

    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    // The route itself has no `:key`, so a real navigation reuses this same
    // component instance and only updates its `hostname` prop - reproduced
    // here the same way, rather than through `$router.push`, since this
    // view is mounted directly with static props rather than under a
    // `<RouterView>` in this test harness.
    await wrapper.setProps({ hostname: 'nonexistent-host' })
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(wrapper.find('.detail-breadcrumb .muted').exists()).toBe(false)
  })
})

// Two agents can share an OS hostname if they're in different domains; the
// page must not silently show one when the caller didn't say which.
describe('AgentDetailView — duplicate hostnames', () => {
  const AGENT_A = { ...mockAgent, id: 1, domain: 'a.example.com', display_name: 'Host A' }
  const AGENT_B = { ...mockAgent, id: 2, domain: 'b.example.com', display_name: 'Host B' }

  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [AGENT_A, AGENT_B] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
  })

  it('shows a picker instead of guessing which agent is meant', async () => {
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('a.example.com')
    expect(wrapper.text()).toContain('b.example.com')
    expect(wrapper.find('.detail-name').exists()).toBe(false)
  })

  const AMBIGUOUS_TEXT = 'More than one host is named'

  it('re-resolves when the domain query changes without the hostname changing', async () => {
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain(AMBIGUOUS_TEXT)

    const router = (wrapper.vm as { $router: { push: (loc: unknown) => Promise<void> } }).$router
    await router.push({ query: { domain: 'a.example.com' } })
    await flushPromises()

    expect(wrapper.text()).not.toContain(AMBIGUOUS_TEXT)
    expect(wrapper.text()).toContain('Host A')
    expect(wrapper.text()).not.toContain('Host B')

    await router.push({ query: { domain: 'b.example.com' } })
    await flushPromises()

    expect(wrapper.text()).not.toContain(AMBIGUOUS_TEXT)
    expect(wrapper.text()).toContain('Host B')
    expect(wrapper.text()).not.toContain('Host A')
  })

  it('clicking a candidate in the picker resolves to that agent', async () => {
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain(AMBIGUOUS_TEXT)

    const candidate = wrapper.findAll('.entity-card').find((c) => c.text().includes('Host B'))
    if (!candidate) throw new Error('no candidate card for Host B')
    await candidate.trigger('click')
    await flushPromises()

    expect(wrapper.text()).not.toContain(AMBIGUOUS_TEXT)
    expect(wrapper.text()).toContain('Host B')
    expect(wrapper.text()).not.toContain('Host A')
  })

  // Regression test: a background refetch never cleared `ambiguousMatches`
  // once it shrank back to a single match, so if the ambiguity resolved
  // itself server-side (e.g. one of the duplicate-hostname agents got
  // merged/deleted from another session) while this page was showing the
  // picker, the picker stayed on screen forever instead of falling through
  // to the now-unambiguous agent.
  it('falls through to the resolved agent once a background refresh clears the ambiguity', async () => {
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain(AMBIGUOUS_TEXT)

    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [AGENT_B] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(wrapper.text()).not.toContain(AMBIGUOUS_TEXT)
    expect(wrapper.text()).toContain('Host B')
  })

  // Regression test: the picker must not get stuck forever if a background
  // refresh goes straight from "ambiguous" to "gone entirely" (e.g. both
  // duplicate-hostname agents were merged/deleted elsewhere between
  // refreshes) - this hits the "not found" throw path directly, skipping
  // the single-match branch the previous test exercises.
  it('clears the picker once a background refresh finds no match at all', async () => {
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain(AMBIGUOUS_TEXT)

    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(wrapper.text()).not.toContain(AMBIGUOUS_TEXT)
    expect(wrapper.find('.entity-card').exists()).toBe(false)
  })

  // Regression test: the reverse transition. If a background refetch turns a
  // previously-resolved single match ambiguous (a duplicate-hostname agent
  // appearing elsewhere), the picker takes over correctly - but `agent`
  // wasn't cleared, so the breadcrumb's domain hint (which reads `agent`
  // directly, outside the template's picker/agent v-else-if chain) kept
  // showing the stale domain next to the hostname while the picker below
  // asked which one was meant.
  it('clears the stale breadcrumb domain hint once a background refresh turns ambiguous', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [AGENT_A] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    expect(wrapper.find('.detail-breadcrumb .muted').text()).toBe('(a.example.com)')

    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [AGENT_A, AGENT_B] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(wrapper.text()).toContain(AMBIGUOUS_TEXT)
    expect(wrapper.find('.detail-breadcrumb .muted').exists()).toBe(false)
  })
})

// A failed run has no archive behind it, so clearing it out is admin-only but
// otherwise reachable through the same header overflow menu as every other
// rare action.
describe('AgentDetailView - clean up failed backups', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  async function renderAsAdmin(): Promise<VueWrapper<ComponentPublicInstance>> {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    return wrapper
  }

  async function openMenu(wrapper: VueWrapper<ComponentPublicInstance>): Promise<void> {
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
  }

  // `mockReports` carries exactly one failed run (id 3).
  it('shows the failed count for an admin', async () => {
    const wrapper = await renderAsAdmin()
    await openMenu(wrapper)

    expect(
      wrapper
        .findAll('.overflow-menu-item')
        .some((i) => i.text() === 'Clean up failed backups (1)'),
    ).toBe(true)
  })

  it('is omitted for a non-admin', async () => {
    setupApi()
    const wrapper = renderWithPlugins(AgentDetailView, { props: { hostname: 'test-host' } })
    await flushPromises()
    await openMenu(wrapper)

    expect(
      wrapper.findAll('.overflow-menu-item').some((i) => i.text().startsWith('Clean up failed')),
    ).toBe(false)
  })

  it('is omitted once there is nothing failed', async () => {
    const onlySuccess = mockReports.filter((r) => r.status !== 'failed')
    setupApi(onlySuccess)
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openMenu(wrapper)

    expect(
      wrapper.findAll('.overflow-menu-item').some((i) => i.text().startsWith('Clean up failed')),
    ).toBe(false)
  })

  // The count backs a menu badge, not the page itself - a failure fetching
  // it must not break the rest of the tab data (regression: it used to sit
  // inside the same Promise.all as the report list, so a rejection there
  // took the whole page down with it).
  it('logs and omits the item when the count fetch fails, without breaking the rest of the page', async () => {
    setupApi()
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents/test-host/reports/failed/count') {
        return Promise.reject(new Error('boom'))
      }
      if (url === '/agents') return Promise.resolve({ data: [mockAgent] })
      if (url === '/agents/test-host/reports') return Promise.resolve({ data: mockReports })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(AgentDetailView, {
      props: { hostname: 'test-host' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()

    expect(wrapper.find('.detail-name').text()).toBe('test-host')
    expect(logger.error).toHaveBeenCalledWith('countFailedReports failed', expect.any(Error))
    await openMenu(wrapper)
    expect(
      wrapper.findAll('.overflow-menu-item').some((i) => i.text().startsWith('Clean up failed')),
    ).toBe(false)
  })

  it('reaches the confirmation through the menu', async () => {
    const wrapper = await renderAsAdmin()
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
    expect(wrapper.text()).toContain('This action cannot be undone.')
  })

  it('closes the dialog without deleting when Cancel is clicked', async () => {
    const wrapper = await renderAsAdmin()
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()

    await wrapper
      .findAll('.modal-footer button')
      .find((b) => b.text().trim() === 'Cancel')!
      .trigger('click')
    await flushPromises()

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('closes the dialog without deleting on dismissal', async () => {
    const wrapper = await renderAsAdmin()
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()

    await dismissModal(wrapper)

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('deletes the failed reports on confirmation and shows a toast', async () => {
    const wrapper = await renderAsAdmin()
    vi.mocked(apiClient.delete).mockResolvedValue({ data: { deleted: 1 } } as never)
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()

    await wrapper
      .findAll('.modal-footer button')
      .find((b) => b.text().trim() === 'Delete failed reports')!
      .trigger('click')
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/test-host/reports/failed', {
      params: {},
    })
    expect(mockToastSuccess).toHaveBeenCalledWith('Deleted 1 failed backup report.')
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('reports a failure and leaves the dialog open', async () => {
    const wrapper = await renderAsAdmin()
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('locked'))
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()

    await wrapper
      .findAll('.modal-footer button')
      .find((b) => b.text().trim() === 'Delete failed reports')!
      .trigger('click')
    await flushPromises()

    expect(mockToastError).toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(1)
  })

  // Regression test: unlike the SSH-key/merge/deploy dialogs, this one isn't
  // gated `v-if="agent"` in the template, so it would otherwise stay open
  // with a stale `failedReportCount` (and silently no-op on delete, since
  // confirmCleanFailedReports returns early once `agent.value` is null) if
  // a background refresh discovers the host is now ambiguous while it's
  // open.
  it('closes the cleanup dialog if a background refresh makes the host ambiguous', async () => {
    const wrapper = await renderAsAdmin()
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()
    expect(openModals(wrapper)).toHaveLength(1)

    const otherAgent = { ...mockAgent, id: 2, domain: 'b.example.com' }
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/agents') return Promise.resolve({ data: [mockAgent, otherAgent] })
      if (String(url).includes('/tags')) return Promise.resolve({ data: [] })
      if (String(url).includes('/hostname-patterns')) return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(wrapper.text()).toContain('More than one host is named')
    expect(openModals(wrapper)).toHaveLength(0)
  })
})
