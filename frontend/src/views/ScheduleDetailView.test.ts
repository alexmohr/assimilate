// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
  },
}))

vi.mock('../components/CronBuilder.vue', () => ({
  default: {
    props: ['modelValue'],
    emits: ['update:modelValue'],
    template:
      '<input class="cron-builder-stub" :value="modelValue" @input="$emit(\'update:modelValue\', $event.target.value)" />',
  },
}))

vi.mock('../components/ToggleSwitch.vue', () => ({
  default: {
    props: ['modelValue'],
    emits: ['update:modelValue'],
    template:
      '<input type="checkbox" class="toggle-switch-stub" :checked="modelValue" @change="$emit(\'update:modelValue\', $event.target.checked)" />',
  },
}))

vi.mock('../components/BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

vi.mock('../utils/cron', () => ({
  cronToHuman: (expr: string): string => `human(${expr})`,
}))

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

// Captured WebSocket message handlers - populated during component setup().
// Accessing wsHandlers here is safe because onMessage is only CALLED during
// component setup(), which happens inside test functions after module evaluation.
const wsHandlers: Record<string, (payload: unknown) => void> = {}

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: (type: string, cb: (p: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
}))

import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import ScheduleDetailView from './ScheduleDetailView.vue'

const mockApiClient = apiClient as {
  get: ReturnType<typeof vi.fn>
  post: ReturnType<typeof vi.fn>
  put: ReturnType<typeof vi.fn>
}

const mockSchedule = {
  id: 1,
  agent_id: 10,
  repo_id: 20,
  schedule_type: 'backup',
  cron_expression: '0 2 * * *',
  enabled: true,
  canary_enabled: false,
  last_run_at: '2026-05-30T02:00:00Z',
  next_run_at: '2026-05-31T02:00:00Z',
  exclude_patterns: ['*.cache', 'node_modules'],
  ignore_global_excludes: false,
  keep_hourly: 24,
  keep_daily: 7,
  keep_weekly: 4,
  keep_monthly: 6,
  keep_yearly: 1,
  compact_enabled: true,
  pre_backup_commands: '["docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"]',
  post_backup_commands: '[]',
}

const mockCheckSchedule = {
  ...mockSchedule,
  id: 2,
  schedule_type: 'check',
  cron_expression: '0 * * * *',
  keep_daily: 0,
  keep_weekly: 0,
  keep_monthly: 0,
  keep_yearly: 0,
  pre_backup_commands: '[]',
  post_backup_commands: '[]',
}

const mockAgents = [
  { id: 10, hostname: 'web-server-01', display_name: 'Web Server' },
  { id: 11, hostname: 'db-server-01', display_name: null },
]

const mockRepos = [
  { id: 20, name: 'server-daily', repo_path: '/repo/daily' },
  { id: 21, name: 'database-hourly', repo_path: '/repo/db' },
]

function setupEditMode(schedule = mockSchedule): void {
  mockApiClient.get.mockImplementation((url: string) => {
    if (url === `/schedules/${schedule.id}`) return Promise.resolve({ data: schedule })
    if (url === `/schedules/${schedule.id}/targets`)
      return Promise.resolve({ data: [{ agent_id: schedule.agent_id, execution_order: 0 }] })
    if (url === `/schedules/${schedule.id}/sources`)
      return Promise.resolve({ data: { backup_sources: ['/data'], backup_sources_per_agent: [] } })
    if (url === '/agents') return Promise.resolve({ data: mockAgents })
    if (url === '/repos') return Promise.resolve({ data: mockRepos })
    return Promise.resolve({ data: [] })
  })
}

function setupCreateMode(): void {
  mockApiClient.get.mockImplementation((url: string) => {
    if (url === '/agents') return Promise.resolve({ data: mockAgents })
    if (url === '/repos') return Promise.resolve({ data: mockRepos })
    return Promise.resolve({ data: [] })
  })
}

describe('ScheduleDetailView - edit mode', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('displays breadcrumb with schedule type', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Schedules')
    expect(wrapper.text()).toContain('Backup')
  })

  it('renders page title with schedule type', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(wrapper.find('h1').text()).toContain('Backup Schedule')
  })

  it('shows agent and repo in info card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Web Server')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('displays next run date in info card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const infoRows = wrapper.findAll('.info-row')
    const nextRunRow = infoRows.find((r) => r.text().includes('Next Run'))
    expect(nextRunRow).toBeTruthy()
    expect(nextRunRow!.text()).not.toContain('\u2014')
  })

  it('displays human-readable cron in info card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(wrapper.text()).toContain('human(0 2 * * *)')
  })

  it('shows retention fields for backup type', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Retention')
    expect(wrapper.text()).toContain('Daily')
    expect(wrapper.text()).toContain('Weekly')
    expect(wrapper.text()).toContain('Monthly')
    expect(wrapper.text()).toContain('Yearly')
  })

  it('has Advanced tab for backup type', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab-btn')
    expect(tabs.some((t) => t.text() === 'Advanced')).toBe(true)
  })

  it('shows and saves the remote rate limit field on the Advanced tab', async () => {
    setupEditMode()
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab-btn')
    await tabs.find((t) => t.text() === 'Advanced')!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Remote Rate Limit')
    const rateLimitInput = wrapper
      .findAll('input[type="number"]')
      .find((i) => i.element.value === '0')
    expect(rateLimitInput).toBeTruthy()

    await rateLimitInput!.setValue(2000)

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save Changes')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({ rate_limit_kbps: 2000 }),
    )
  })

  it('does not show Advanced tab for check type', async () => {
    setupEditMode(mockCheckSchedule)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '2' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab-btn')
    expect(tabs.some((t) => t.text() === 'Advanced')).toBe(false)
  })

  it('shows Save Changes button', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save Changes')
    expect(saveBtn).toBeTruthy()
  })

  it('calls PUT on save', async () => {
    setupEditMode()
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save Changes')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith('/schedules/1', expect.any(Object))
  })

  it('shows error banner on load failure', async () => {
    mockApiClient.get.mockRejectedValue(new Error('Not found'))
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '999' } })
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
  })

  it('shows save success message after successful save', async () => {
    setupEditMode()
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save Changes')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(wrapper.find('.save-success').exists()).toBe(true)
    expect(wrapper.text()).toContain('Saved')
  })

  it('shows save error when schedule is null (edit mode)', async () => {
    mockApiClient.get.mockRejectedValue(new Error('Load failed'))
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '999' } })
    await flushPromises()

    // The save() null-guard is defensive - the save bar is hidden when schedule
    // is null (v-if="schedule || isCreate" wraps the form). Test the ref directly.
    const vm = wrapper.vm as { save: () => Promise<void>; saveError: string | null }
    expect(vm.saveError).toBeNull()

    await vm.save()
    await flushPromises()

    // Previously schedule.value!.id would throw. The guard now produces a
    // friendly error message instead.
    expect(vm.saveError).toBe('Schedule not found')
  })

  it('shows save error on PUT failure', async () => {
    setupEditMode()
    mockApiClient.put.mockRejectedValue(new Error('Server error'))
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save Changes')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(wrapper.find('.error-inline').exists()).toBe(true)
  })

  it('shows em dash for null next_run_at', async () => {
    setupEditMode({ ...mockSchedule, next_run_at: null })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const infoRows = wrapper.findAll('.info-row')
    const nextRunRow = infoRows.find((r) => r.text().includes('Next Run'))
    expect(nextRunRow!.text()).toContain('\u2014')
  })

  it('shows em dash for null last_run_at', async () => {
    setupEditMode({ ...mockSchedule, last_run_at: null })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const infoRows = wrapper.findAll('.info-row')
    const lastRunRow = infoRows.find((r) => r.text().includes('Last Run'))
    expect(lastRunRow!.text()).toContain('\u2014')
  })

  it('shows weekly retention schedule for weekly backup config', async () => {
    const weeklySchedule = {
      ...mockSchedule,
      cron_expression: '0 3 * * 0',
      keep_daily: 0,
      keep_weekly: 52,
      keep_monthly: 12,
      keep_yearly: 5,
    }
    setupEditMode(weeklySchedule)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const retentionGrid = wrapper.find('.retention-grid')
    expect(retentionGrid.exists()).toBe(true)
    const inputs = retentionGrid.findAll('input[type="number"]')
    const weeklyInput = inputs[2]
    expect(weeklyInput.element.value).toBe('52')
  })

  it('shows Run Now and no Cancel Backup button when nothing is running', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const buttons = wrapper.findAll('button').map((b) => b.text())
    expect(buttons).toContain('Run Now')
    expect(buttons).not.toContain('Cancel Backup')
  })

  it('seeds running state from recent reports and shows Cancel Backup instead of Run Now', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({ data: [{ agent_id: mockSchedule.agent_id, execution_order: 0 }] })
      if (url === '/schedules/1/sources')
        return Promise.resolve({ data: { backup_sources: ['/data'], backup_sources_per_host: [] } })
      if (url === '/schedules/1/reports')
        return Promise.resolve({ data: [{ id: 1, status: 'started' }] })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const buttons = wrapper.findAll('button').map((b) => b.text())
    expect(buttons).toContain('Cancel Backup')
    expect(buttons).not.toContain('Run Now')
  })

  it('calls cancel API when Cancel Backup is clicked', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({ data: [{ agent_id: mockSchedule.agent_id, execution_order: 0 }] })
      if (url === '/schedules/1/sources')
        return Promise.resolve({ data: { backup_sources: ['/data'], backup_sources_per_host: [] } })
      if (url === '/schedules/1/reports')
        return Promise.resolve({ data: [{ id: 1, status: 'pending' }] })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const cancelBtn = wrapper.findAll('button').find((b) => b.text() === 'Cancel Backup')
    await cancelBtn!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith('/schedules/1/cancel')
  })
})

describe('ScheduleDetailView - create mode', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders New Schedule title', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    expect(wrapper.find('h1').text()).toContain('New Schedule')
  })

  it('shows breadcrumb with New', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    expect(wrapper.text()).toContain('New')
  })

  it('shows agent and repo dropdowns', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Select agents...')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('shows schedule type selector', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Schedule Type')
    expect(wrapper.text()).toContain('Integrity Check')
    expect(wrapper.text()).toContain('Verify (extract dry-run)')
  })

  it('shows Create Schedule button', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    const createBtn = wrapper.findAll('button').find((b) => b.text() === 'Create Schedule')
    expect(createBtn).toBeTruthy()
  })

  it('does not show info card in create mode', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    expect(wrapper.find('.info-card').exists()).toBe(false)
  })
})

describe('ScheduleDetailView - WebSocket handlers', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    for (const key of Object.keys(wsHandlers)) {
      delete wsHandlers[key]
    }
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('BackupStarted with matching schedule_id shows BACKUP IN PROGRESS card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: 'server-daily-2026-06-26',
      schedule_id: 1,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
    expect(wrapper.text()).toContain('Backup in progress')
  })

  it('BackupStarted with non-matching schedule_id does not activate progress card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 999,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupStarted with null schedule_id and matching repo_id activates progress card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: null,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
  })

  it('BackupCompleted with matching schedule_id hides BACKUP IN PROGRESS card', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()
    expect(wrapper.find('.live-log-card').exists()).toBe(true)

    wsHandlers['BackupCompleted']?.({
      hostname: 'web-server-01',
      target_name: 'server-daily',
      report: { repo_id: 20 },
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupLog with matching schedule_id and archive_progress JSON updates progress data', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: 1,
      repo_id: 20,
      line: JSON.stringify({
        type: 'archive_progress',
        nfiles: 1234,
        original_size: 5368709120,
        path: '/home/user/important.txt',
      }),
    })
    await nextTick()

    expect(wrapper.find('.live-log-empty').exists()).toBe(false)
    expect(wrapper.text()).toContain('1,234')
    expect(wrapper.text()).toContain('/home/user/important.txt')
  })

  it('replayed BackupLog updates a running backup after reload', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({ data: [{ agent_id: mockSchedule.agent_id, execution_order: 0 }] })
      if (url === '/schedules/1/sources')
        return Promise.resolve({ data: { backup_sources: ['/data'], backup_sources_per_host: [] } })
      if (url === '/schedules/1/reports')
        return Promise.resolve({
          data: [
            {
              id: 1,
              status: 'started',
              started_at: '2026-06-27T10:00:00Z',
              agent_id: 10,
              original_size: 0,
            },
          ],
        })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
    expect(wrapper.find('.live-log-empty').exists()).toBe(true)

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: 1,
      repo_id: 20,
      line: JSON.stringify({
        type: 'archive_progress',
        nfiles: 321,
        original_size: 4096,
        path: '/srv/data.tar',
      }),
    })
    await nextTick()

    expect(wrapper.find('.live-log-empty').exists()).toBe(false)
    expect(wrapper.text()).toContain('321')
    expect(wrapper.text()).toContain('/srv/data.tar')
  })

  it('BackupLog with wrong schedule_id does not update progress', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: 999,
      repo_id: 20,
      line: JSON.stringify({
        type: 'archive_progress',
        nfiles: 1,
        original_size: 100,
        path: '/tmp/file',
      }),
    })
    await nextTick()

    expect(wrapper.find('.live-log-empty').exists()).toBe(true)
  })

  it('BackupLog with null schedule_id and matching repo_id updates progress', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: null,
      repo_id: 20,
      line: JSON.stringify({
        type: 'archive_progress',
        nfiles: 500,
        original_size: 1073741824,
        path: '',
      }),
    })
    await nextTick()

    expect(wrapper.find('.live-log-empty').exists()).toBe(false)
    expect(wrapper.text()).toContain('500')
  })

  it('BackupLog with null schedule_id and wrong repo_id does not update progress', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: null,
      repo_id: 999,
      line: JSON.stringify({
        type: 'archive_progress',
        nfiles: 100,
        original_size: 100,
        path: '/tmp/file',
      }),
    })
    await nextTick()

    expect(wrapper.find('.live-log-empty').exists()).toBe(true)
  })

  it('BackupLog with non-JSON line adds text to live log output', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      repo_id: 20,
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: 1,
      repo_id: 20,
      line: 'Creating archive server-daily-2026-06-26...',
    })
    await nextTick()

    expect(wrapper.find('.live-log-output').exists()).toBe(true)
    expect(wrapper.text()).toContain('Creating archive server-daily-2026-06-26...')
  })
})
