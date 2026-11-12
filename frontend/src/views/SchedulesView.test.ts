// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())

const wsHandlers = vi.hoisted(() => ({}) as Record<string, (payload: unknown) => void>)
vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: (type: string, cb: (payload: unknown) => void) => void } => ({
    onMessage: (type, cb) => {
      wsHandlers[type] = cb
    },
  }),
}))

vi.mock('../composables/useMobile', () => ({
  useMobile: (): { isMobile: boolean } => ({ isMobile: false }),
}))

vi.mock('../utils/cron', () => ({
  cronToHuman: (expr: string): string => `human(${expr})`,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn() },
}))

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
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

vi.mock('../components/BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

vi.mock('../components/EmptyState.vue', () => ({
  default: {
    props: ['icon', 'title', 'description', 'action'],
    emits: ['action'],
    template: '<div class="empty-state"><slot /><span>{{ title }}</span></div>',
  },
}))

import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import SchedulesView from './SchedulesView.vue'

const mockApiClient = apiClient as {
  get: ReturnType<typeof vi.fn>
  post: ReturnType<typeof vi.fn>
  put: ReturnType<typeof vi.fn>
  delete: ReturnType<typeof vi.fn>
}

const mockSchedules = [
  {
    id: 1,
    agent_id: 10,
    repo_id: 20,
    schedule_type: 'backup',
    cron_expression: '0 2 * * *',
    enabled: true,
    canary_enabled: false,
    last_run_at: '2026-05-30T02:00:00Z',
    next_run_at: '2026-05-31T02:00:00Z',
    exclude_patterns: [],
    ignore_global_excludes: false,
    keep_daily: 7,
    keep_weekly: 4,
    keep_monthly: 6,
    keep_yearly: 1,
    compact_enabled: true,
    pre_backup_commands: [],
    post_backup_commands: [],
    hook_timeout_seconds: 60,

    on_failure: 'continue',
    target_hostnames: ['web-server-01', 'db-server-01'],
  },
  {
    id: 2,
    agent_id: 11,
    repo_id: 21,
    schedule_type: 'check',
    cron_expression: '0 * * * *',
    enabled: true,
    canary_enabled: false,
    last_run_at: '2026-05-30T01:00:00Z',
    next_run_at: '2026-05-31T01:00:00Z',
    exclude_patterns: [],
    ignore_global_excludes: false,
    keep_daily: 0,
    keep_weekly: 0,
    keep_monthly: 0,
    keep_yearly: 0,
    compact_enabled: false,
    pre_backup_commands: [],
    post_backup_commands: [],
    hook_timeout_seconds: 60,

    on_failure: 'stop',
    target_hostnames: ['db-server-01'],
  },
  {
    id: 3,
    agent_id: 12,
    repo_id: 22,
    schedule_type: 'backup',
    cron_expression: '0 3 * * 0',
    enabled: false,
    canary_enabled: false,
    last_run_at: null,
    next_run_at: null,
    exclude_patterns: [],
    ignore_global_excludes: false,
    keep_daily: 0,
    keep_weekly: 52,
    keep_monthly: 12,
    keep_yearly: 5,
    compact_enabled: true,
    pre_backup_commands: [],
    post_backup_commands: [],
    hook_timeout_seconds: 60,

    on_failure: 'continue',
    target_hostnames: ['media-store-01'],
  },
]

const mockAgents = [
  { id: 10, hostname: 'web-server-01', display_name: 'Web Server' },
  { id: 11, hostname: 'db-server-01', display_name: null },
  { id: 12, hostname: 'media-store-01', display_name: 'Media Store' },
]

const mockRepos = [
  {
    id: 20,
    name: 'server-daily',
    repo_path: '/repo/daily',
    ssh_host: 'borg-backup.example.com',
    enabled: true,
  },
  {
    id: 21,
    name: 'database-hourly',
    repo_path: '/repo/db',
    ssh_host: 'nas.example.com',
    enabled: true,
  },
  {
    id: 22,
    name: 'media-weekly',
    repo_path: '/repo/media',
    ssh_host: 'nas.example.com',
    enabled: true,
  },
]

const mockHealth = [
  {
    repo_id: 20,
    schedule_id: 1,
    hostname: 'web-server-01',
    target_name: 'server-daily',
    last_status: 'success',
    last_backup_at: '2026-05-30T02:00:00Z',
    is_overdue: false,
    last_error_message: null,
    cron_expression: '0 2 * * *',
    schedule_enabled: true,
    consecutive_missed_backups: 0,
    missed_backup_threshold: 3,
  },
  {
    repo_id: 21,
    schedule_id: 2,
    hostname: 'db-server-01',
    target_name: 'database-hourly',
    last_status: 'failed',
    last_backup_at: '2026-05-29T01:00:00Z',
    is_overdue: true,
    last_error_message: 'Connection refused',
    cron_expression: '0 * * * *',
    schedule_enabled: true,
    consecutive_missed_backups: 0,
    missed_backup_threshold: 3,
  },
]

const overdueWebServerHealth = [
  {
    repo_id: 20,
    schedule_id: 1,
    hostname: 'web-server-01',
    target_name: 'server-daily',
    last_status: 'success',
    last_backup_at: '2026-05-25T02:00:00Z',
    is_overdue: true,
    last_error_message: null,
    cron_expression: '0 2 * * *',
    schedule_enabled: true,
    consecutive_missed_backups: 0,
    missed_backup_threshold: 3,
  },
]

const offlineWebServerAgents = [
  {
    id: 10,
    hostname: 'web-server-01',
    display_name: 'Web Server',
    is_connected: false,
    last_seen_at: '2026-05-23T02:00:00Z',
  },
  {
    id: 11,
    hostname: 'db-server-01',
    display_name: null,
    is_connected: true,
    last_seen_at: '2026-05-30T02:00:00Z',
  },
]

function setupApiSuccess(): void {
  mockApiClient.get.mockImplementation((url: string) => {
    // Cloned so a toggle handler mutating the returned array (schedules.value[i] = ...)
    // can't leak state into later tests that share this same mockSchedules array.
    if (url === '/schedules') return Promise.resolve({ data: mockSchedules.map((s) => ({ ...s })) })
    if (url === '/repos') return Promise.resolve({ data: mockRepos })
    if (url === '/agents') return Promise.resolve({ data: mockAgents })
    if (url === '/stats/health') return Promise.resolve({ data: mockHealth })
    return Promise.resolve({ data: [] })
  })
}

type Wrapper = ReturnType<typeof renderWithPlugins>

/** Clicks one option of the toolbar's "Group:" segmented control. */
async function selectGroupMode(wrapper: Wrapper, label: string): Promise<void> {
  const option = wrapper.findAll('.segmented-option').find((o) => o.text() === label)
  expect(option, `no "${label}" group option`).toBeDefined()
  await option!.trigger('click')
  await flushPromises()
}

function groupTitles(wrapper: Wrapper): string[] {
  return wrapper.findAll('.list-group-title').map((t) => t.text())
}

function groupFor(wrapper: Wrapper, title: string): ReturnType<Wrapper['find']> | undefined {
  return wrapper.findAll('.list-group').find((g) => g.find('.list-group-title').text() === title)
}

/** The row of the schedule whose name (or, unnamed, repository) is `name`. */
function rowFor(wrapper: Wrapper, name: string): ReturnType<Wrapper['find']> | undefined {
  return wrapper.findAll('.list-row').find((r) => r.find('.list-row-name').text() === name)
}

/** The one health label of a row; absent on a healthy row, whose dot says it alone. */
function healthChip(wrapper: Wrapper, name: string): ReturnType<Wrapper['find']> {
  return rowFor(wrapper, name)!.find('.schedules-health')
}

/** Unfolds the Disabled group, which starts collapsed to its header. */
async function openDisabledGroup(wrapper: Wrapper): Promise<void> {
  await groupFor(wrapper, 'Disabled')!.find('.schedules-group-toggle').trigger('click')
  await flushPromises()
}

describe('SchedulesView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('renders a row per schedule, named after its repository when unnamed', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await openDisabledGroup(wrapper)

    expect(wrapper.findAll('.list-row')).toHaveLength(3)
    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).toContain('media-weekly')
  })

  /// A pending catch-up is the row's health when nothing is wrong with it.
  it('flags a schedule with a run waiting on a host to come back', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: mockSchedules.map((s) => ({
            ...s,
            catch_up_pending_count: s.id === 1 ? 1 : 0,
          })),
        })
      }
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: mockHealth })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const labels = wrapper.findAll('.schedules-health--info')
    expect(labels).toHaveLength(1)
    expect(labels[0].text()).toBe('Catch-up pending')
    expect(labels[0].attributes('title')).toContain('One host missed a run')
    expect(rowFor(wrapper, 'server-daily')!.find('.status-dot--info').exists()).toBe(true)
    expect(rowFor(wrapper, 'server-daily')!.find('.schedules-health').text()).toBe(
      'Catch-up pending',
    )
  })

  it('shows no catch-up badge when nothing is pending', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).not.toContain('Catch-up pending')
  })

  /**
   * The streak is cleared on the server as soon as a run reaches its host, and
   * a scheduled start sends no DataChanged - so the list has to re-read health
   * when a backup starts, or the chip lingers until the whole run finishes.
   */
  it('drops the missed chip when a backup starts and the streak is cleared', async () => {
    let missed = 1
    setupApiSuccess()
    const base = mockApiClient.get.getMockImplementation() as (url: string) => Promise<unknown>
    mockApiClient.get.mockImplementation((url: string) =>
      url === '/stats/health'
        ? Promise.resolve({
            data: mockHealth.map((h) =>
              h.schedule_id === 1 ? { ...h, consecutive_missed_backups: missed } : h,
            ),
          })
        : base(url),
    )
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    expect(wrapper.text()).toContain('1/3 missed')

    missed = 0
    wsHandlers['BackupStarted']?.({ hostname: 'web-server-01', schedule_id: 1 })
    await flushPromises()

    expect(wrapper.text()).not.toContain('1/3 missed')
  })

  it('names the first agent and counts the rest, listing them all on hover', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const agent = rowFor(wrapper, 'server-daily')!.find('.schedules-agent')
    expect(agent.text()).toBe('web-server-01 +1')
    expect(agent.attributes('title')).toBe('Web Server (web-server-01), db-server-01')
    expect(rowFor(wrapper, 'database-hourly')!.find('.schedules-agent').text()).toBe('db-server-01')
  })

  it('shows a healthy schedule by its dot alone and names Disabled for a disabled one', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await openDisabledGroup(wrapper)

    // The green dot says healthy; no label repeats it.
    expect(healthChip(wrapper, 'server-daily').exists()).toBe(false)
    expect(rowFor(wrapper, 'server-daily')!.find('.status-dot--success').exists()).toBe(true)
    expect(rowFor(wrapper, 'server-daily')!.text()).not.toContain('Healthy')

    const disabled = healthChip(wrapper, 'media-weekly')
    expect(disabled.text()).toBe('Disabled')
    expect(disabled.classes()).toContain('schedules-health--neutral')
    // A neutral state keeps the dot's own grey.
    expect(rowFor(wrapper, 'media-weekly')!.find('.status-dot').classes()).toEqual(['status-dot'])
    // A disabled schedule has no next run to show.
    expect(rowFor(wrapper, 'media-weekly')!.text()).toContain('\u2014')
  })

  it('links each row to its schedule', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const row = rowFor(wrapper, 'server-daily')!
    expect(row.element.tagName).toBe('A')
    expect(row.attributes('href')).toBe('/schedules/1')
    expect(rowFor(wrapper, 'database-hourly')!.attributes('href')).toBe('/schedules/2')
  })

  it('renders schedule type badges', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Backup')
    expect(wrapper.text()).toContain('Integrity check')
  })

  it('displays human-readable cron expression', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(rowFor(wrapper, 'server-daily')!.find('.list-row-sub').text()).toContain(
      'human(0 2 * * *)',
    )
  })

  it('shows one status dot per row and at most one health label, never a pill', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await openDisabledGroup(wrapper)

    const rows = wrapper.findAll('.list-row')
    expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) {
      expect(row.findAll('.status-dot')).toHaveLength(1)
      expect(row.findAll('.schedules-health').length).toBeLessThanOrEqual(1)
      expect(row.findAll('.badge')).toHaveLength(0)
    }
  })

  it('combines Overdue and Failed into one danger chip for a schedule that is both', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const chip = healthChip(wrapper, 'database-hourly')
    expect(chip.text()).toBe('Overdue · Failed')
    expect(chip.classes()).toContain('schedules-health--danger')
    expect(rowFor(wrapper, 'database-hourly')!.find('.status-dot--danger').exists()).toBe(true)
  })

  it("shows the failed run's error message in the chip's tooltip", async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(healthChip(wrapper, 'database-hourly').attributes('title')).toContain(
      'Connection refused',
    )
  })

  it("links a failed run's label to that run's output in the Activity log", async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    const push = vi.spyOn(wrapper.vm.$router, 'push')

    const chip = healthChip(wrapper, 'database-hourly')
    expect(chip.attributes('role')).toBe('link')
    expect(chip.attributes('tabindex')).toBe('0')
    expect(chip.classes()).toContain('schedules-health--link')
    await chip.trigger('click')

    // Only the log opens, not the schedule the row itself links to.
    expect(push).toHaveBeenCalledTimes(1)
    expect(push).toHaveBeenCalledWith({
      path: '/activity',
      query: { category: 'backup', schedule_id: '2', status: 'failed' },
    })
  })

  it("opens a warned run's output from the keyboard too", async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: mockSchedules })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') {
        return Promise.resolve({
          data: [
            { ...mockHealth[0], last_status: 'warning', last_error_message: 'file vanished' },
            mockHealth[1],
          ],
        })
      }
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    const push = vi.spyOn(wrapper.vm.$router, 'push')

    const chip = healthChip(wrapper, 'server-daily')
    expect(chip.text()).toBe('Warning')
    expect(chip.attributes('title')).toBe('file vanished')
    await chip.trigger('keydown', { key: 'Enter' })

    expect(push).toHaveBeenCalledWith({
      path: '/activity',
      query: { category: 'backup', schedule_id: '1', status: 'warning' },
    })
  })

  it('shows an Overdue chip whose tooltip lists per-host detail', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: [mockSchedules[0]] })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: overdueWebServerHealth })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const chip = healthChip(wrapper, 'server-daily')
    expect(chip.text()).toBe('Overdue')
    expect(chip.classes()).toContain('schedules-health--warning')
    expect(rowFor(wrapper, 'server-daily')!.find('.status-dot--warning').exists()).toBe(true)
    // Overdue alone is not a run that failed, so it does not link to a run's output.
    expect(chip.attributes('role')).toBeUndefined()
    expect(chip.attributes('title')).toContain('Web Server (web-server-01) — last backup:')
    expect(wrapper.text()).not.toContain('Last backup failed')
    // The next run takes the warning tone, and the group counts it.
    expect(rowFor(wrapper, 'server-daily')!.find('.schedules-next--overdue').exists()).toBe(true)
    expect(wrapper.find('.list-group-aside').text()).toBe('1 overdue')
  })

  it('shows an agent-offline note in the Overdue tooltip for a disconnected agent', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: [mockSchedules[0]] })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: offlineWebServerAgents })
      if (url === '/stats/health') return Promise.resolve({ data: overdueWebServerHealth })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(healthChip(wrapper, 'server-daily').attributes('title')).toContain(
      'agent offline (last seen',
    )
  })

  it('says Agent offline for an otherwise healthy schedule whose agent is disconnected', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: [mockSchedules[0]] })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: offlineWebServerAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [mockHealth[0]] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const chip = healthChip(wrapper, 'server-daily')
    expect(chip.text()).toBe('Agent offline')
    expect(chip.attributes('title')).toContain('Web Server (web-server-01) - last seen')
  })

  it('does not show a next-run time as overdue when the schedule is on track', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(rowFor(wrapper, 'server-daily')!.find('.schedules-next--overdue').exists()).toBe(false)
  })

  it('shows empty state when no schedules exist', async () => {
    mockApiClient.get.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.text()).toContain('No schedules configured')
  })

  it('shows error banner on API failure', async () => {
    mockApiClient.get.mockRejectedValue(new Error('Network error'))
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(wrapper.text()).toContain('Failed to load schedules.')
  })

  it('filters by enabled status', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const selects = wrapper.findAll('select')
    const statusSelect = selects.find((s) => s.find('option[value="enabled"]').exists())
    expect(statusSelect).toBeTruthy()
    await statusSelect!.setValue('enabled')
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).not.toContain('media-weekly')
  })

  it('filters by schedule type', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const selects = wrapper.findAll('select')
    const typeSelect = selects.find((s) => s.find('option[value="check"]').exists())
    expect(typeSelect).toBeTruthy()
    await typeSelect!.setValue('check')
    await flushPromises()

    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).not.toContain('server-daily')
  })

  it('filters by text search', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const searchInput = wrapper.find('input.search-input')
    await searchInput.setValue('server-daily')
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).not.toContain('database-hourly')
  })

  it('filters by the storage host with a bare term', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    await wrapper.find('input.search-input').setValue('borg-backup')
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).not.toContain('database-hourly')
  })

  it('scopes a host: term to the storage host, not the agent', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const searchInput = wrapper.find('input.search-input')
    await searchInput.setValue('host:nas')
    await flushPromises()

    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).toContain('media-weekly')
    expect(wrapper.text()).not.toContain('server-daily')

    // db-server-01 is an agent, so scoping it to host matches nothing.
    await searchInput.setValue('host:db-server')
    await flushPromises()
    expect(wrapper.text()).not.toContain('database-hourly')
  })

  it('scopes an agent: term to the target agents', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    await wrapper.find('input.search-input').setValue('agent:media-store')
    await flushPromises()

    expect(wrapper.text()).toContain('media-weekly')
    expect(wrapper.text()).not.toContain('server-daily')
  })

  it('keeps a schedule matching either side of a pipe', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    await wrapper.find('input.search-input').setValue('agent:media-store | host:borg-backup')
    await flushPromises()

    expect(wrapper.text()).toContain('media-weekly')
    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).not.toContain('database-hourly')
  })

  it('requires both space-separated terms to match', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const searchInput = wrapper.find('input.search-input')
    await searchInput.setValue('host:nas agent:media-store')
    await flushPromises()

    expect(wrapper.text()).toContain('media-weekly')
    expect(wrapper.text()).not.toContain('database-hourly')
  })

  it('filters by health status: success', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const selects = wrapper.findAll('select')
    const healthSelect = selects.find((s) => s.find('option[value="failed"]').exists())
    expect(healthSelect).toBeTruthy()
    await healthSelect!.setValue('success')
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).not.toContain('database-hourly')
    expect(wrapper.text()).not.toContain('media-weekly')
  })

  it('filters by health status: failed', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const selects = wrapper.findAll('select')
    const healthSelect = selects.find((s) => s.find('option[value="failed"]').exists())
    await healthSelect!.setValue('failed')
    await flushPromises()

    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).not.toContain('server-daily')
  })

  it('filters by health status: warning', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: mockSchedules })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') {
        return Promise.resolve({
          data: [{ ...mockHealth[0], last_status: 'warning' }, mockHealth[1]],
        })
      }
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const selects = wrapper.findAll('select')
    const healthSelect = selects.find((s) => s.find('option[value="failed"]').exists())
    await healthSelect!.setValue('warning')
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).not.toContain('database-hourly')
  })

  // Running, triggering and pausing a schedule live on its detail page
  // (ScheduleDetailView.test.ts covers Run now, Cancel backup and the Enabled
  // switch); a row is a link to that page and carries no controls of its own.
  it('offers no run, cancel or enable control on the rows', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const rows = wrapper.findAll('.list-row')
    expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) {
      expect(row.findAll('button')).toHaveLength(0)
      expect(row.find('[role="switch"]').exists()).toBe(false)
    }
  })

  it('shows a Running chip while a run of the schedule is in flight', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: mockSchedules })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') {
        return Promise.resolve({
          data: [{ ...mockHealth[0], last_status: 'started' }, mockHealth[1]],
        })
      }
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const chip = healthChip(wrapper, 'server-daily')
    expect(chip.text()).toBe('Running')
    expect(chip.classes()).toContain('schedules-health--accent')
    expect(rowFor(wrapper, 'server-daily')!.find('.status-dot--accent').exists()).toBe(true)
    expect(healthChip(wrapper, 'database-hourly').text()).not.toBe('Running')
  })

  it('has New button linking to /schedules/new', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const newLink = wrapper.find('a[href="/schedules/new"]')
    expect(newLink.exists()).toBe(true)
  })

  it('page title is Schedules', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('h1').text()).toBe('Schedules')
  })

  it('groups schedules into time-based sections, with disabled schedules in Disabled', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    // The two enabled schedules' next_run_at is in the past relative to any
    // real test-run clock, so both land in "Due now"; the disabled one is
    // always "Disabled" regardless of its next_run_at.
    const titles = groupTitles(wrapper)
    expect(titles).toContain('Due now')
    expect(titles).toContain('Disabled')

    const dueNowGroup = groupFor(wrapper, 'Due now')
    expect(dueNowGroup!.find('.list-group-count').text()).toBe('2')
    expect(dueNowGroup!.text()).toContain('server-daily')
    expect(dueNowGroup!.text()).toContain('database-hourly')

    await openDisabledGroup(wrapper)
    expect(groupFor(wrapper, 'Disabled')!.text()).toContain('media-weekly')
  })

  it('folds the Disabled group to its header until it is opened', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const group = groupFor(wrapper, 'Disabled')!
    const toggle = group.find('.schedules-group-toggle')
    expect(toggle.attributes('aria-expanded')).toBe('false')
    expect(group.find('.list-group-count').text()).toBe('1')
    expect(group.findAll('.list-row')).toHaveLength(0)
    expect(group.find('.list-cols').exists()).toBe(false)

    await toggle.trigger('click')
    expect(groupFor(wrapper, 'Disabled')!.findAll('.list-row')).toHaveLength(1)
    expect(toggle.attributes('aria-expanded')).toBe('true')

    await toggle.trigger('click')
    expect(groupFor(wrapper, 'Disabled')!.findAll('.list-row')).toHaveLength(0)

    // The other groups never fold.
    expect(groupFor(wrapper, 'Due now')!.find('.schedules-group-toggle').exists()).toBe(false)
  })

  it('shows the disabled rows without a click while searching or filtering to them', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    await wrapper.find('input.search-input').setValue('media')
    await flushPromises()
    expect(groupFor(wrapper, 'Disabled')!.findAll('.list-row')).toHaveLength(1)

    await wrapper.find('input.search-input').setValue('')
    const statusSelect = wrapper
      .findAll('select')
      .find((s) => s.find('option[value="enabled"]').exists())!
    await statusSelect.setValue('disabled')
    await flushPromises()
    expect(groupFor(wrapper, 'Disabled')!.findAll('.list-row')).toHaveLength(1)
  })

  it('heads each group with the column names once', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const cols = groupFor(wrapper, 'Due now')!.findAll('.list-cols')
    expect(cols).toHaveLength(1)
    expect(cols[0].text()).toBe('ScheduleAgentHealthNext run')
  })

  it('buckets schedules into Next 24 hours, This week, and Later by next_run_at', async () => {
    const at = (hours: number): string => new Date(Date.now() + hours * 3600_000).toISOString()
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            { ...mockSchedules[0], next_run_at: at(12) }, // > 6h, <= 24h -> Next 24 hours
            { ...mockSchedules[1], next_run_at: at(72) }, // > 24h, <= 7d -> This week
            { ...mockSchedules[2], enabled: true, next_run_at: at(24 * 10) }, // > 7d -> Later
          ],
        })
      }
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(groupFor(wrapper, 'Next 24 hours')!.text()).toContain('server-daily')
    expect(groupFor(wrapper, 'This week')!.text()).toContain('database-hourly')
    expect(groupFor(wrapper, 'Later')!.text()).toContain('media-weekly')
  })

  it('groups schedules by agent, listing a multi-agent schedule under each target', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await selectGroupMode(wrapper, 'Agent')

    // Sections are titled with the same "Display name (hostname)" label the
    // filter matches on, and an agent without a display name keeps its bare
    // hostname.
    expect(groupTitles(wrapper)).toEqual([
      'db-server-01',
      'Media Store (media-store-01)',
      'Web Server (web-server-01)',
    ])

    // server-daily targets both web-server-01 and db-server-01, so it belongs
    // to both sections - grouping by agent answers "what backs up this
    // machine", which one arbitrary section could not.
    expect(groupFor(wrapper, 'Web Server (web-server-01)')!.text()).toContain('server-daily')
    const dbGroup = groupFor(wrapper, 'db-server-01')!
    expect(dbGroup.text()).toContain('server-daily')
    expect(dbGroup.text()).toContain('database-hourly')
    expect(dbGroup.find('.list-group-count').text()).toBe('2')

    // Disabled schedules are no longer split off into their own bucket, so the
    // disabled one sits with its agent and keeps its Disabled chip.
    expect(groupFor(wrapper, 'Media Store (media-store-01)')!.text()).toContain('media-weekly')
    expect(healthChip(wrapper, 'media-weekly').text()).toBe('Disabled')
  })

  it('lists a schedule once per agent section even when a hostname repeats', async () => {
    // Two agents can report the same hostname from different domains
    // (`agents_hostname_domain_idx` makes only (hostname, domain) unique), so
    // a schedule targeting both carries that hostname twice. They share one
    // section here, and the card belongs in it once.
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [{ ...mockSchedules[1], target_hostnames: ['db-server-01', 'db-server-01'] }],
        })
      }
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await selectGroupMode(wrapper, 'Agent')

    expect(groupTitles(wrapper)).toEqual(['db-server-01'])
    const group = groupFor(wrapper, 'db-server-01')!
    expect(group.findAll('.list-row')).toHaveLength(1)
    expect(group.find('.list-group-count').text()).toBe('1')
  })

  it('labels an agent section shared by two same-hostname agents without picking one', async () => {
    // `edge-proxy` is two machines told apart only by domain. A schedule names
    // the hostname alone, so the view cannot tell which of them a schedule
    // targets: both land in one section, which must not be titled with either
    // agent's display name and must say that it covers more than one agent.
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            { ...mockSchedules[0], target_hostnames: ['edge-proxy'] },
            { ...mockSchedules[1], target_hostnames: ['edge-proxy'] },
          ],
        })
      }
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') {
        return Promise.resolve({
          data: [
            { id: 20, hostname: 'edge-proxy', display_name: 'DC1 edge', domain: 'dc1.example.com' },
            { id: 21, hostname: 'edge-proxy', display_name: 'DC2 edge', domain: 'dc2.example.com' },
          ],
        })
      }
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await selectGroupMode(wrapper, 'Agent')

    expect(groupTitles(wrapper)).toEqual(['edge-proxy'])
    const group = groupFor(wrapper, 'edge-proxy')!
    expect(group.text()).not.toContain('DC1 edge')
    expect(group.text()).not.toContain('DC2 edge')
    const badge = group.find('.list-group-header .badge')
    expect(badge.text()).toBe('2 agents')
    expect(badge.attributes('title')).toContain('different domains')
  })

  it('groups schedules by repository', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await selectGroupMode(wrapper, 'Repo')

    expect(groupTitles(wrapper)).toEqual(['database-hourly', 'media-weekly', 'server-daily'])
    for (const title of ['database-hourly', 'media-weekly', 'server-daily']) {
      expect(groupFor(wrapper, title)!.find('.list-group-count').text()).toBe('1')
    }
  })

  it('buckets schedules with no agent or no repository into their own section', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            { ...mockSchedules[0], repo_id: null, target_hostnames: [] },
            { ...mockSchedules[1] },
          ],
        })
      }
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    await selectGroupMode(wrapper, 'Agent')
    // The fallback section sorts last, after every named agent.
    expect(groupTitles(wrapper)).toEqual(['db-server-01', 'No agents'])

    await selectGroupMode(wrapper, 'Repo')
    expect(groupTitles(wrapper)).toEqual(['database-hourly', 'No repository'])
  })

  it('renames the fallback section when a repository is named after it', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            { ...mockSchedules[0], repo_id: 20 },
            { ...mockSchedules[1], repo_id: null },
          ],
        })
      }
      if (url === '/repos') {
        return Promise.resolve({ data: [{ ...mockRepos[0], name: 'No repository' }] })
      }
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await selectGroupMode(wrapper, 'Repo')

    // The repository keeps the name its owner gave it; the keyless section is
    // the one that gives way, so the two headings stay distinguishable.
    expect(groupTitles(wrapper)).toEqual(['No repository', 'No repository (none assigned)'])
  })

  it('returns to the time buckets when the group mode is switched back', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    await selectGroupMode(wrapper, 'Repo')
    expect(groupTitles(wrapper)).not.toContain('Disabled')

    await selectGroupMode(wrapper, 'Time')
    expect(groupTitles(wrapper)).toContain('Disabled')
  })

  // Two schedules whose next runs land `gapMinutes` apart (five by default).
  // The second one's agent and repository and the first one's recent run
  // durations are what each test varies.
  function mockOverlapSchedules({
    secondRepoId = 20,
    secondAgents = ['db-server-01'],
    firstDurations = [] as number[],
    gapMinutes = 5,
  } = {}): void {
    const soon = (minutes: number): string => new Date(Date.now() + minutes * 60_000).toISOString()
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            {
              ...mockSchedules[0],
              name: 'Web nightly',
              repo_id: 20,
              target_hostnames: ['web-server-01'],
              next_run_at: soon(120),
            },
            {
              ...mockSchedules[1],
              name: 'DB nightly',
              repo_id: secondRepoId,
              target_hostnames: secondAgents,
              next_run_at: soon(120 + gapMinutes),
            },
          ],
        })
      }
      if (url === '/repos') {
        return Promise.resolve({
          data: [
            { ...mockRepos[0], ssh_host: 'shared-box.example.com' },
            { ...mockRepos[1], ssh_host: 'shared-box.example.com' },
          ],
        })
      }
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      if (url.startsWith('/stats/activity')) {
        return Promise.resolve({
          data: firstDurations.map((duration, i) => ({
            id: 500 + i,
            started_at: `2026-05-2${i}T02:00:00Z`,
            duration_secs: duration,
            status: 'success',
            schedule_id: 1,
            repo_id: 20,
          })),
        })
      }
      return Promise.resolve({ data: [] })
    })
  }

  it('warns when two agents write to one repository at overlapping times', async () => {
    // A median of 30 minutes keeps schedule 1 busy when schedule 2 starts.
    mockOverlapSchedules({ firstDurations: [1200, 1800, 2400] })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const notices = wrapper.findAll('.notice-bar')
    expect(notices).toHaveLength(1)
    expect(notices[0].find('.notice-bar-title').text()).toBe(
      'Two agents write to repo server-daily at the same time',
    )
    expect(notices[0].text()).toMatch(
      /web-server-01 runs \d\d:\d\d – ~\d\d:\d\d \(estimated from recent runs\), db-server-01 starts \d\d:\d\d\. The repo lock makes one of them wait or fail\./,
    )
    expect(notices[0].attributes('href')).toBe('/repos/20?tab=schedules')

    expect(healthChip(wrapper, 'Web nightly').text()).toBe('Overlaps db-server-01')
    expect(healthChip(wrapper, 'DB nightly').text()).toBe('Overlaps web-server-01')
  })

  it('does not warn about a run that starts as the default-length one before it ends', async () => {
    // Five minutes apart with the five-minute default: touching, not overlapping.
    mockOverlapSchedules()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    expect(wrapper.find('.notice-bar').exists()).toBe(false)
  })

  it('names the assumed run length when there is no history to estimate from', async () => {
    // Two minutes apart: inside the five-minute default window.
    mockOverlapSchedules({ gapMinutes: 2 })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.notice-bar').text()).toContain('(assumed 5 minutes, no recent runs)')
  })

  it('does not warn about runs into different repositories on one storage host', async () => {
    mockOverlapSchedules({ secondRepoId: 21, firstDurations: [3600] })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.notice-bar').exists()).toBe(false)
    expect(healthChip(wrapper, 'Web nightly').text()).not.toContain('Overlaps')
  })

  it('does not warn about two schedules on the same agent', async () => {
    mockOverlapSchedules({ secondAgents: ['web-server-01'], firstDurations: [3600] })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.notice-bar').exists()).toBe(false)
  })

  it('counts a repository a schedule only names in its run history', async () => {
    // Schedule 2's primary repository differs, but its recent runs show it
    // also writes into server-daily.
    mockOverlapSchedules({ secondRepoId: 21, firstDurations: [3600] })
    const base = mockApiClient.get.getMockImplementation() as (url: string) => Promise<unknown>
    mockApiClient.get.mockImplementation((url: string) =>
      url.startsWith('/stats/activity')
        ? (base(url) as Promise<{ data: unknown[] }>).then((res) => ({
            data: [
              ...res.data,
              {
                id: 900,
                started_at: '2026-05-29T02:00:00Z',
                duration_secs: 60,
                status: 'success',
                schedule_id: 2,
                repo_id: 20,
              },
            ],
          }))
        : base(url),
    )
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.notice-bar-title').text()).toContain('repo server-daily')
    expect(rowFor(wrapper, 'DB nightly')!.find('.list-row-sub').text()).toContain(
      'database-hourly, server-daily',
    )
  })

  it('lists rows that need attention first within their group', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            { ...mockSchedules[0], target_hostnames: ['db-server-01'] },
            { ...mockSchedules[1] },
          ],
        })
      }
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [mockHealth[1]] })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    // Both target db-server-01, so the default agent sort ties and would keep
    // server-daily first; the failing database-hourly moves ahead of it.
    const names = wrapper.findAll('.list-row-name').map((n) => n.text())
    expect(names).toEqual(['database-hourly', 'server-daily'])
  })

  describe('counted filter pills', () => {
    function pills(wrapper: Wrapper): string[] {
      return wrapper.findAll('.filter-pill').map((p) => p.text())
    }

    it('counts all, overdue, failing and disabled schedules', async () => {
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      expect(pills(wrapper)).toEqual(['All · 3', 'Overdue · 1', 'Failing · 1', 'Disabled · 1'])
      expect(wrapper.find('.filter-pill.active').text()).toBe('All · 3')
    })

    it('filters the list to a pill and back to everything', async () => {
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()
      const pill = (label: string): ReturnType<Wrapper['find']> =>
        wrapper.findAll('.filter-pill').find((p) => p.text().startsWith(label))!

      await pill('Failing').trigger('click')
      expect(wrapper.findAll('.list-row-name').map((n) => n.text())).toEqual(['database-hourly'])
      expect(pill('Failing').attributes('aria-pressed')).toBe('true')

      await pill('Disabled').trigger('click')
      expect(wrapper.findAll('.list-row-name').map((n) => n.text())).toEqual(['media-weekly'])

      await pill('Overdue').trigger('click')
      expect(wrapper.findAll('.list-row-name').map((n) => n.text())).toEqual(['database-hourly'])

      await pill('All').trigger('click')
      expect(wrapper.findAll('.list-row')).toHaveLength(2)
    })

    it('marks no pill when the selects ask for something no pill stands for', async () => {
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      const typeSelect = wrapper
        .findAll('select')
        .find((s) => s.find('option[value="check"]').exists())!
      await typeSelect.setValue('check')

      expect(wrapper.find('.filter-pill.active').exists()).toBe(false)
    })
  })

  it('caps the activity fetch at the recent runs kept per schedule, relying on the backend to cap per schedule', async () => {
    let activityUrl: string | null = null
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules')
        return Promise.resolve({ data: mockSchedules.map((s) => ({ ...s })) })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      if (url.startsWith('/stats/activity')) {
        activityUrl = url
        return Promise.resolve({ data: [] })
      }
      return Promise.resolve({ data: [] })
    })
    renderWithPlugins(SchedulesView)
    await flushPromises()

    // The backend applies this limit per schedule_id (not to the result set
    // overall), so a flat 10 is correct regardless of how many schedules exist.
    expect(activityUrl).toBe('/stats/activity?days=30&limit_per_schedule=10')
  })

  it('still renders the schedules list when the activity feed request fails', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules')
        return Promise.resolve({ data: mockSchedules.map((s) => ({ ...s })) })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: [] })
      if (url.startsWith('/stats/activity')) return Promise.reject(new Error('network error'))
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(false)
    expect(rowFor(wrapper, 'server-daily')!.attributes('href')).toBe('/schedules/1')
    expect(rowFor(wrapper, 'database-hourly')).toBeDefined()
  })

  describe('persisted view settings', () => {
    it('restores the group mode from a previous visit', async () => {
      localStorage.setItem('assimilate-schedules-group', 'agent')
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      const active = wrapper
        .findAll('.segmented-option')
        .find((o) => o.classes().includes('active'))
      expect(active?.text()).toBe('Agent')
    })

    it('restores the sort field and direction from a previous visit', async () => {
      localStorage.setItem('assimilate-schedules-sort-field', 'next_run')
      localStorage.setItem('assimilate-schedules-sort-dir', 'desc')
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      const active = wrapper
        .findAll('.sort-controls button')
        .find((b) => b.classes().includes('active'))
      expect(active?.text()).toContain('Next run')
    })

    it('restores the status filter from a previous visit', async () => {
      localStorage.setItem('assimilate-schedules-filter-status', 'disabled')
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      const selects = wrapper.findAll('select')
      const statusSelect = selects.find((s) => s.find('option[value="enabled"]').exists())
      expect(statusSelect!.element.value).toBe('disabled')
    })

    it('restores the type filter from a previous visit', async () => {
      localStorage.setItem('assimilate-schedules-filter-type', 'check')
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      const selects = wrapper.findAll('select')
      const typeSelect = selects.find((s) => s.find('option[value="check"]').exists())
      expect(typeSelect!.element.value).toBe('check')
    })

    it('restores the health filter from a previous visit', async () => {
      localStorage.setItem('assimilate-schedules-filter-health', 'warning')
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      const selects = wrapper.findAll('select')
      const healthSelect = selects.find((s) => s.find('option[value="failed"]').exists())
      expect(healthSelect!.element.value).toBe('warning')
    })

    it('reacts to a filter query arriving after the page has already mounted', async () => {
      // Distinct from "restores the health filter from a previous visit"
      // above: that one covers the initial-mount override path, this one
      // covers the reactive watcher that handles an in-app navigation (a
      // dashboard link clicked while Schedules is already open) landing on a
      // live component. Mirrors HostsView.test.ts's identical coverage-query
      // regression test.
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView, { routeOverrides: '/schedules' })
      await flushPromises()

      const selects = wrapper.findAll('select')
      const healthSelect = selects.find((s) => s.find('option[value="failed"]').exists())
      expect(healthSelect!.element.value).toBe('all')

      const router = (wrapper.vm as unknown as { $router: { push: (to: string) => Promise<void> } })
        .$router
      await router.push('/schedules?filter=overdue')
      await flushPromises()

      expect(healthSelect!.element.value).toBe('overdue')
    })

    it('persists a group mode change for the next visit', async () => {
      setupApiSuccess()
      const wrapper = renderWithPlugins(SchedulesView)
      await flushPromises()

      await selectGroupMode(wrapper, 'Repo')

      expect(localStorage.getItem('assimilate-schedules-group')).toBe('repo')
    })
  })
})

describe('SchedulesView sorting', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  /**
   * Schedule 1 (server-daily) and schedule 2 (database-hourly) both target
   * db-server-01, so that section holds the pair; rows are titled by repo.
   * No health is reported, so neither row is a problem that would sort first.
   */
  function sharedAgentOrder(wrapper: Wrapper): string[] {
    return groupFor(wrapper, 'db-server-01')!
      .findAll('.list-row-name')
      .map((n) => n.text())
  }

  async function sortBy(wrapper: Wrapper, label: string): Promise<void> {
    const button = wrapper.findAll('.sort-controls button').find((b) => b.text() === label)
    expect(button, `no "${label}" sort button`).toBeDefined()
    await button!.trigger('click')
    await flushPromises()
  }

  async function renderByAgent(): Promise<Wrapper> {
    setupApiSuccess()
    const base = mockApiClient.get.getMockImplementation() as (url: string) => Promise<unknown>
    mockApiClient.get.mockImplementation((url: string) =>
      url === '/stats/health' ? Promise.resolve({ data: [] }) : base(url),
    )
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()
    await selectGroupMode(wrapper, 'Agent')
    return wrapper
  }

  it('orders schedules by type, backups before checks', async () => {
    const wrapper = await renderByAgent()
    // The default agent sort puts the db-server-01-only check first.
    expect(sharedAgentOrder(wrapper)).toEqual(['database-hourly', 'server-daily'])

    await sortBy(wrapper, 'Type')

    expect(sharedAgentOrder(wrapper)).toEqual(['server-daily', 'database-hourly'])
  })

  it('orders schedules by last run, oldest first and then newest first', async () => {
    const wrapper = await renderByAgent()

    await sortBy(wrapper, 'Last run')
    // Schedule 2 last ran at 01:00, an hour before schedule 1.
    expect(sharedAgentOrder(wrapper)).toEqual(['database-hourly', 'server-daily'])

    await sortBy(wrapper, 'Last run')
    expect(sharedAgentOrder(wrapper)).toEqual(['server-daily', 'database-hourly'])
  })
})
