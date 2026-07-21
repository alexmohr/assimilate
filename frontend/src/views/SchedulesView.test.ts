// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn> } => ({
    onMessage: vi.fn(),
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
    pre_backup_commands: '',
    post_backup_commands: '',

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
    pre_backup_commands: '',
    post_backup_commands: '',

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
    pre_backup_commands: '',
    post_backup_commands: '',

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
  { id: 20, name: 'server-daily', repo_path: '/repo/daily', enabled: true },
  { id: 21, name: 'database-hourly', repo_path: '/repo/db', enabled: true },
  { id: 22, name: 'media-weekly', repo_path: '/repo/media', enabled: true },
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
  },
]

function setupApiSuccess(): void {
  mockApiClient.get.mockImplementation((url: string) => {
    if (url === '/schedules') return Promise.resolve({ data: mockSchedules })
    if (url === '/repos') return Promise.resolve({ data: mockRepos })
    if (url === '/agents') return Promise.resolve({ data: mockAgents })
    if (url === '/stats/health') return Promise.resolve({ data: mockHealth })
    return Promise.resolve({ data: [] })
  })
}

describe('SchedulesView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('renders schedule cards with repo name', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).toContain('database-hourly')
    expect(wrapper.text()).toContain('media-weekly')
  })

  it('shows target hosts on the schedule card', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Web Server (web-server-01), db-server-01')
    expect(wrapper.text()).toContain('1 host')
    expect(wrapper.text()).toContain('2 hosts')
  })

  it('shows enabled/disabled badge', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('Enabled')
    expect(text).toContain('Disabled')
  })

  it('renders schedule type badges', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Backup')
    expect(wrapper.text()).toContain('Integrity Check')
  })

  it('displays human-readable cron expression', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('human(0 2 * * *)')
  })

  it('shows health badge for success status', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Success')
  })

  it('shows overdue health badge', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Overdue')
  })

  it('shows last backup failed error toggle when error message present', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Last backup failed')
  })

  it('shows an overdue toggle with per-host detail when a host has no error message', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: [mockSchedules[0]] })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: overdueWebServerHealth })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('1 host overdue')
    expect(wrapper.text()).not.toContain('Last backup failed')

    const toggle = wrapper.findAll('.error-toggle').find((b) => b.text().includes('overdue'))
    expect(toggle).toBeTruthy()
    expect(wrapper.text()).not.toContain('Web Server (web-server-01) — last backup:')

    await toggle!.trigger('click')

    expect(wrapper.text()).toContain('Web Server (web-server-01) — last backup:')
  })

  it('shows an agent-offline note for an overdue host whose agent is disconnected', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: [mockSchedules[0]] })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') {
        return Promise.resolve({
          data: [
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
          ],
        })
      }
      if (url === '/stats/health') return Promise.resolve({ data: overdueWebServerHealth })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const toggle = wrapper.findAll('.error-toggle').find((b) => b.text().includes('overdue'))
    await toggle!.trigger('click')

    expect(wrapper.text()).toContain('agent offline (last seen')
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

  it('calls run now API on run button click and shows success toast', async () => {
    setupApiSuccess()
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const runButtons = wrapper.findAll('button').filter((b) => b.text() === 'Run')
    expect(runButtons.length).toBeGreaterThan(0)
    await runButtons[0].trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      expect.stringMatching(/^\/schedules\/\d+\/run$/),
      {},
    )
    expect(mockToastSuccess).toHaveBeenCalledWith(expect.stringMatching(/started/i))
  })

  it('shows error toast when run now API fails', async () => {
    setupApiSuccess()
    mockApiClient.post.mockRejectedValue({ response: { data: { error: 'agent offline' } } })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const runButtons = wrapper.findAll('button').filter((b) => b.text() === 'Run')
    expect(runButtons.length).toBeGreaterThan(0)
    await runButtons[0].trigger('click')
    await flushPromises()

    expect(mockToastError).toHaveBeenCalled()
  })

  it('shows Cancel instead of Run when the schedule is currently running', async () => {
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

    const buttons = wrapper.findAll('button')
    expect(buttons.some((b) => b.text() === 'Cancel')).toBe(true)
    // Schedule 1 (running) no longer shows a Run button; schedules 2 and 3 still do.
    expect(buttons.filter((b) => b.text() === 'Run')).toHaveLength(2)
  })

  it('calls cancel API on cancel button click for a running schedule', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: mockSchedules })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') {
        return Promise.resolve({ data: [{ ...mockHealth[0], last_status: 'started' }] })
      }
      return Promise.resolve({ data: [] })
    })
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const cancelButton = wrapper.findAll('button').find((b) => b.text() === 'Cancel')
    expect(cancelButton).toBeTruthy()
    await cancelButton!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith('/schedules/1/cancel')
    expect(mockToastSuccess).toHaveBeenCalledWith(expect.stringMatching(/cancel/i))
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
})
