// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())

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
    // Cloned so a toggle handler mutating the returned array (schedules.value[i] = ...)
    // can't leak state into later tests that share this same mockSchedules array.
    if (url === '/schedules') return Promise.resolve({ data: mockSchedules.map((s) => ({ ...s })) })
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

  it('shows the agent count on the schedule card without the raw agent list', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('1 agent')
    expect(wrapper.text()).toContain('2 agents')
    expect(wrapper.text()).not.toContain('Web Server (web-server-01), db-server-01')
  })

  it('shows nothing for an enabled schedule and a Disabled pill for a disabled one', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const cards = wrapper.findAll('.entity-card')
    const enabledCard = cards.find((c) => c.text().includes('server-daily'))
    const disabledCard = cards.find((c) => c.text().includes('media-weekly'))

    expect(enabledCard!.find('.entity-status-pill').exists()).toBe(false)
    expect(enabledCard!.classes()).not.toContain('entity-card--notable')

    expect(disabledCard!.find('.entity-status-pill').text()).toBe('Disabled')
    expect(disabledCard!.classes()).toContain('entity-card--notable')
  })

  it('opens a schedule when its card is clicked', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('server-daily'))
    await card!.trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toBe('/schedules/1')
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

    expect(wrapper.text()).toContain('human(0 2 * * *)')
  })

  it('shows no badge row at all for a healthy, non-overdue schedule', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const healthyCard = wrapper
      .findAll('.entity-card')
      .find((c) => c.text().includes('server-daily'))
    expect(healthyCard!.find('.entity-badge-row').exists()).toBe(false)
  })

  it('shows overdue health badge', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.text()).toContain('Overdue')
  })

  it('shows both Overdue and Failed chips for a schedule that is both', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('database-hourly'))
    expect(card!.find('.entity-issue-chip.sev-warning').text()).toBe('Overdue')
    expect(card!.find('.entity-issue-chip.sev-danger').text()).toBe('Failed')
  })

  it("shows the failed run's error message in the Failed chip's tooltip", async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('database-hourly'))
    expect(card!.find('.entity-issue-chip.sev-danger').attributes('title')).toBe(
      'Connection refused',
    )
  })

  it('navigates to the activity log filtered to this schedule when the Failed chip is clicked', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('database-hourly'))
    await card!.find('.entity-issue-chip.sev-danger').trigger('click')
    await flushPromises()

    const router = (
      wrapper.vm as unknown as {
        $router: { currentRoute: { value: { path: string; query: Record<string, string> } } }
      }
    ).$router
    expect(router.currentRoute.value.path).toBe('/activity')
    expect(router.currentRoute.value.query).toMatchObject({
      category: 'backup',
      schedule_id: '2',
      status: 'failed',
    })
  })

  it('navigates to the schedule detail page when the Overdue chip is clicked', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('database-hourly'))
    await card!.find('.entity-issue-chip.sev-warning').trigger('click')
    await flushPromises()

    const router = (
      wrapper.vm as unknown as { $router: { currentRoute: { value: { path: string } } } }
    ).$router
    expect(router.currentRoute.value.path).toBe('/schedules/2')
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

    const chip = wrapper.find('.entity-issue-chip.sev-warning')
    expect(chip.text()).toBe('Overdue')
    expect(chip.attributes('title')).toContain('Web Server (web-server-01) — last backup:')
    expect(wrapper.text()).not.toContain('Last backup failed')
  })

  it('shows an agent-offline note in the Overdue tooltip for a disconnected agent', async () => {
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

    const chip = wrapper.find('.entity-issue-chip.sev-warning')
    expect(chip.attributes('title')).toContain('agent offline (last seen')
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

  // The two places that trigger a run word this type differently, so each
  // keeps its own label rather than the shared composable picking one.
  it('names a verify schedule by its full wording when started', async () => {
    setupApiSuccess()
    const verifySchedule = { ...mockSchedules[0], schedule_type: 'verify' }
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') return Promise.resolve({ data: [verifySchedule] })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: mockHealth })
      return Promise.resolve({ data: [] })
    })
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const runButton = wrapper.findAll('button').find((b) => b.text() === 'Run')
    await runButton!.trigger('click')
    await flushPromises()

    expect(mockToastSuccess).toHaveBeenCalledWith('Verify started.')
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
    expect(wrapper.find('.entity-running-pill').exists()).toBe(true)
    expect(wrapper.text()).toContain('Running')
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

  it('disables an enabled schedule via the card toggle and shows a toast', async () => {
    setupApiSuccess()
    mockApiClient.put.mockResolvedValue({ data: { ...mockSchedules[0], enabled: false } })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('server-daily'))
    expect(card!.find('.schedule-toggle-label').text()).toBe('Enabled')

    await card!.find('button[role="switch"]').trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({ cron_expression: '0 2 * * *', enabled: false }),
    )
    expect(mockToastSuccess).toHaveBeenCalledWith(expect.stringMatching(/disabled/i))
  })

  it('enables a disabled schedule via the card toggle', async () => {
    setupApiSuccess()
    mockApiClient.put.mockResolvedValue({ data: { ...mockSchedules[2], enabled: true } })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('media-weekly'))
    expect(card!.find('.schedule-toggle-label').text()).toBe('Disabled')

    await card!.find('button[role="switch"]').trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/3',
      expect.objectContaining({ enabled: true }),
    )
    expect(mockToastSuccess).toHaveBeenCalledWith(expect.stringMatching(/enabled/i))

    // Once enabled, the schedule moves from the Paused group into a
    // time-based one (its next_run_at is null, so "Unscheduled"), which
    // remounts its card in a different .schedule-grid - re-query rather than
    // reuse the pre-toggle `card` reference, which now points at a detached
    // node. The badge disappears and the label updates.
    const updatedCard = wrapper
      .findAll('.entity-card')
      .find((c) => c.text().includes('media-weekly'))
    expect(updatedCard!.find('.schedule-toggle-label').text()).toBe('Enabled')
    expect(updatedCard!.find('.entity-status-pill').exists()).toBe(false)
  })

  it('shows an error toast when toggling a schedule fails', async () => {
    setupApiSuccess()
    mockApiClient.put.mockRejectedValue({ response: { data: { error: 'update failed' } } })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('server-daily'))
    await card!.find('button[role="switch"]').trigger('click')
    await flushPromises()

    expect(mockToastError).toHaveBeenCalled()
    // The card keeps its prior state since the update was rejected.
    expect(card!.find('.schedule-toggle-label').text()).toBe('Enabled')
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

  it('groups schedules into time-based sections, with disabled schedules in Paused', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    // The two enabled schedules' next_run_at is in the past relative to any
    // real test-run clock, so both land in "Due now"; the disabled one is
    // always "Paused" regardless of its next_run_at.
    const titles = wrapper.findAll('.schedule-group-title').map((t) => t.text())
    expect(titles).toContain('Due now')
    expect(titles).toContain('Paused')

    const dueNowGroup = wrapper
      .findAll('.schedule-group')
      .find((g) => g.find('.schedule-group-title').text() === 'Due now')
    expect(dueNowGroup!.find('.schedule-group-count').text()).toBe('2')
    expect(dueNowGroup!.text()).toContain('server-daily')
    expect(dueNowGroup!.text()).toContain('database-hourly')

    const pausedGroup = wrapper
      .findAll('.schedule-group')
      .find((g) => g.find('.schedule-group-title').text() === 'Paused')
    expect(pausedGroup!.text()).toContain('media-weekly')
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

    const groupFor = (title: string) =>
      wrapper
        .findAll('.schedule-group')
        .find((g) => g.find('.schedule-group-title').text() === title)

    expect(groupFor('Next 24 hours')!.text()).toContain('server-daily')
    expect(groupFor('This week')!.text()).toContain('database-hourly')
    expect(groupFor('Later')!.text()).toContain('media-weekly')
  })

  it('navigates to the schedule detail page when a card is clicked', async () => {
    setupApiSuccess()
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('server-daily'))
    await card!.trigger('click')
    await flushPromises()

    const router = (
      wrapper.vm as unknown as { $router: { currentRoute: { value: { path: string } } } }
    ).$router
    expect(router.currentRoute.value.path).toBe('/schedules/1')
  })

  it('shows the 24h collision rail for schedules due soon, flagging same-host collisions', async () => {
    const soon = (hours: number): string => new Date(Date.now() + hours * 3600_000).toISOString()
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules') {
        return Promise.resolve({
          data: [
            { ...mockSchedules[0], repo_id: 20, next_run_at: soon(2) },
            { ...mockSchedules[1], repo_id: 21, next_run_at: soon(2.1) },
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
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    expect(wrapper.find('.timeline-rail').exists()).toBe(true)
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(2)
    expect(wrapper.find('.timeline-note').text()).toContain('shared-box.example.com')
  })

  it('renders the run-history strip for a schedule from the activity feed', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules')
        return Promise.resolve({ data: mockSchedules.map((s) => ({ ...s })) })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/stats/health') return Promise.resolve({ data: mockHealth })
      if (url.startsWith('/stats/activity?days=30&limit_per_schedule=')) {
        return Promise.resolve({
          data: [
            {
              id: 101,
              started_at: '2026-05-30T02:00:00Z',
              duration_secs: 300,
              status: 'success',
              schedule_id: 1,
            },
            {
              id: 102,
              started_at: '2026-05-29T02:00:00Z',
              duration_secs: 45,
              status: 'failed',
              schedule_id: 1,
            },
          ],
        })
      }
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(SchedulesView)
    await flushPromises()

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('server-daily'))
    expect(card!.findAll('.run-bar')).toHaveLength(2)
    expect(card!.text()).toContain('2 runs · 1 failed')

    const otherCard = wrapper
      .findAll('.entity-card')
      .find((c) => c.text().includes('database-hourly'))
    expect(otherCard!.text()).toContain('No runs yet')
  })

  it('caps the activity fetch at the run-history bar count, relying on the backend to cap per schedule', async () => {
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

    const card = wrapper.findAll('.entity-card').find((c) => c.text().includes('server-daily'))
    expect(card!.text()).toContain('No runs yet')
  })
})
