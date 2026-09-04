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
    delete: vi.fn(),
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

vi.mock('../components/ArchiveFileBrowser.vue', () => ({
  default: {
    props: ['repoId', 'archive', 'isAdmin'],
    template: '<div class="archive-file-browser-stub" />',
  },
}))

vi.mock('../utils/cron', () => ({
  cronToHuman: (expr: string): string => `human(${expr})`,
}))

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
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
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import { hookCommand } from '../utils/hookCommands'
import ScheduleDetailView from './ScheduleDetailView.vue'
import { logger } from '../utils/logger'

const mockApiClient = apiClient as {
  get: ReturnType<typeof vi.fn>
  post: ReturnType<typeof vi.fn>
  put: ReturnType<typeof vi.fn>
  delete: ReturnType<typeof vi.fn>
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
  pre_backup_commands: [hookCommand('docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql')],
  post_backup_commands: [],
  hook_timeout_seconds: 60,
  missed_backup_threshold: 3,
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
  pre_backup_commands: [],
  post_backup_commands: [],
}

const mockVerifySchedule = {
  ...mockSchedule,
  id: 3,
  schedule_type: 'verify',
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

async function createEditWrapper(): Promise<ReturnType<typeof renderWithPlugins>> {
  setupEditMode()
  const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
  await flushPromises()
  return wrapper
}

function setupEditModeWithReport(report: Record<string, unknown>): void {
  mockApiClient.get.mockImplementation((url: string) => {
    if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
    if (url === '/schedules/1/targets')
      return Promise.resolve({ data: [{ agent_id: mockSchedule.agent_id, execution_order: 0 }] })
    if (url === '/schedules/1/sources')
      return Promise.resolve({ data: { backup_sources: ['/data'], backup_sources_per_host: [] } })
    if (url === '/schedules/1/reports') return Promise.resolve({ data: [report] })
    if (url === '/schedules/1/reports/failed/count') {
      const count = report.status === 'failed' ? 1 : 0
      return Promise.resolve({ data: { count } })
    }
    if (url === '/agents') return Promise.resolve({ data: mockAgents })
    if (url === '/repos') return Promise.resolve({ data: mockRepos })
    return Promise.resolve({ data: [] })
  })
}

async function renderAndStartBackup(): Promise<ReturnType<typeof renderWithPlugins>> {
  setupEditMode()
  const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
  await flushPromises()

  wsHandlers['BackupStarted']?.({
    hostname: 'web-server-01',
    target_name: 'server-daily',
    archive_name: null,
    schedule_id: 1,
    started_at: new Date().toISOString(),
  })
  await nextTick()
  return wrapper
}

/** Switches to the Settings tab, where the save bar and its form live. */
async function goToSettings(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
  await wrapper
    .findAll('.tab')
    .find((t) => t.text() === 'Settings')!
    .trigger('click')
  await flushPromises()
}

/** Switches to a settings sub-nav section; assumes the Settings tab is already active. */
async function goToSection(
  wrapper: ReturnType<typeof renderWithPlugins>,
  label: string,
): Promise<void> {
  await wrapper
    .findAll('.settings-nav-item')
    .find((b) => b.text() === label)!
    .trigger('click')
  await flushPromises()
}

/** Reads a `dt`/`dd` pair out of the Overview tab's info-grid by its label. */
function infoValueFor(wrapper: ReturnType<typeof renderWithPlugins>, label: string): string {
  const dts = wrapper.findAll('.info-grid dt')
  const dt = dts.find((d) => d.text() === label)
  return dt!.element.nextElementSibling!.textContent ?? ''
}

async function renderEditModeAndSave(): Promise<ReturnType<typeof renderWithPlugins>> {
  setupEditMode()
  mockApiClient.put.mockResolvedValue({ data: mockSchedule })
  const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
  await flushPromises()
  await goToSettings(wrapper)

  const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save changes')
  await saveBtn!.trigger('click')
  await flushPromises()
  return wrapper
}
describe('ScheduleDetailView - edit mode', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('displays breadcrumb with schedule type', async () => {
    const wrapper = await createEditWrapper()

    expect(wrapper.text()).toContain('Schedules')
    expect(wrapper.text()).toContain('Backup')
  })

  it('renders the header with the schedule type when it has no name', async () => {
    const wrapper = await createEditWrapper()

    expect(wrapper.find('h1').text()).toBe('Backup')
  })

  it('labels a verify-type schedule correctly and hides the Backups tab', async () => {
    setupEditMode(mockVerifySchedule)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '3' } })
    await flushPromises()

    expect(wrapper.find('h1').text()).toBe('Verify (extract dry-run)')
    expect(wrapper.findAll('.tab').some((t) => t.text() === 'Backups')).toBe(false)
  })

  it('opens on the Overview tab and shows agent and repo in the info summary', async () => {
    const wrapper = await createEditWrapper()

    expect(wrapper.text()).toContain('Web Server')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('shows the next run date in the info summary', async () => {
    const wrapper = await createEditWrapper()

    expect(infoValueFor(wrapper, 'Next run')).not.toContain('—')
  })

  it('shows a human-readable cron in the info summary', async () => {
    const wrapper = await createEditWrapper()

    expect(wrapper.text()).toContain('human(0 2 * * *)')
  })

  it('shows an em dash for a null next_run_at', async () => {
    setupEditMode({ ...mockSchedule, next_run_at: null })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(infoValueFor(wrapper, 'Next run')).toContain('—')
  })

  it('shows Never for a null last_run_at', async () => {
    setupEditMode({ ...mockSchedule, last_run_at: null })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(infoValueFor(wrapper, 'Last run')).toContain('Never')
  })

  it('shows retention fields under Settings > Retention for a backup schedule', async () => {
    const wrapper = await createEditWrapper()
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Retention')

    expect(wrapper.text()).toContain('Daily')
    expect(wrapper.text()).toContain('Weekly')
    expect(wrapper.text()).toContain('Monthly')
  })

  it('shows weekly retention values on the Retention section', async () => {
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
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Retention')

    const retentionGrid = wrapper.find('.retention-grid')
    expect(retentionGrid.exists()).toBe(true)
    const inputs = retentionGrid.findAll('input[type="number"]')
    const weeklyInput = inputs[2]
    expect(weeklyInput.element.value).toBe('52')
  })

  it('has an Advanced section under Settings for a backup schedule', async () => {
    const wrapper = await createEditWrapper()
    await goToSettings(wrapper)

    const sections = wrapper.findAll('.settings-nav-item').map((b) => b.text())
    expect(sections).toContain('Advanced')
  })

  it('does not show an Advanced section for a check-type schedule', async () => {
    setupEditMode(mockCheckSchedule)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '2' } })
    await flushPromises()
    await goToSettings(wrapper)

    const sections = wrapper.findAll('.settings-nav-item').map((b) => b.text())
    expect(sections).not.toContain('Advanced')
  })

  it('shows and saves the remote rate limit field on the Advanced section', async () => {
    setupEditMode()
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Advanced')

    expect(wrapper.text()).toContain('Remote rate limit')
    const rateLimitInput = wrapper
      .findAll('input[type="number"]')
      .find((i) => i.element.value === '0')
    expect(rateLimitInput).toBeTruthy()

    await rateLimitInput!.setValue(2000)

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save changes')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({ rate_limit_kbps: 2000 }),
    )
  })

  it('names a verify schedule by what it does, not by its type word', async () => {
    // "Verify" alone reads as a status check; the page says what borg is
    // actually asked to do, and the heading falls back to it when the
    // schedule has no name.
    setupEditMode(mockVerifySchedule)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '3' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Verify (extract dry-run)')
  })

  it('does not show Advanced tab for check type', async () => {
    // Advanced is a Settings sub-nav section now rather than a top-level tab,
    // so it must never appear in the tab strip - for any schedule type.
    setupEditMode(mockCheckSchedule)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '2' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Advanced')).toBe(false)
  })

  it('shows Save changes only on the Settings tab', async () => {
    const wrapper = await createEditWrapper()

    expect(wrapper.find('.save-bar').exists()).toBe(false)
    await goToSettings(wrapper)
    expect(wrapper.findAll('button').find((b) => b.text() === 'Save changes')).toBeTruthy()
  })

  it('calls PUT on save', async () => {
    await renderEditModeAndSave()

    expect(mockApiClient.put).toHaveBeenCalledWith('/schedules/1', expect.any(Object))
  })

  it('shows error banner on load failure', async () => {
    mockApiClient.get.mockRejectedValue(new Error('Not found'))
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '999' } })
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
  })

  it('shows save success message after successful save', async () => {
    const wrapper = await renderEditModeAndSave()

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
    await goToSettings(wrapper)

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save changes')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(wrapper.find('.error-inline').exists()).toBe(true)
  })

  it('shows Run now and no Cancel backup button when nothing is running', async () => {
    const wrapper = await createEditWrapper()

    const buttons = wrapper.findAll('button').map((b) => b.text())
    expect(buttons).toContain('Run now')
    expect(buttons).not.toContain('Cancel backup')
  })

  it('clicking Run now triggers the schedule with an empty body', async () => {
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = await createEditWrapper()

    const runNowButton = wrapper.findAll('button').find((b) => b.text() === 'Run now')
    expect(runNowButton).toBeTruthy()
    await runNowButton!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith('/schedules/1/run', {})
  })

  it('shows an Overdue badge and Retry button for an overdue target, with an offline note', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({
          data: [
            { agent_id: 10, execution_order: 0 },
            { agent_id: 11, execution_order: 1 },
          ],
        })
      if (url === '/schedules/1/sources')
        return Promise.resolve({
          data: { backup_sources: ['/data'], backup_sources_per_agent: [] },
        })
      if (url === '/agents')
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
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      if (url === '/stats/health')
        return Promise.resolve({
          data: [
            {
              repo_id: 20,
              schedule_id: 1,
              hostname: 'web-server-01',
              target_name: 'server-daily',
              last_status: 'success',
              last_backup_at: '2026-05-23T02:00:00Z',
              is_overdue: true,
              last_error_message: null,
              cron_expression: '0 2 * * *',
              schedule_enabled: true,
            },
            {
              repo_id: 20,
              schedule_id: 1,
              hostname: 'db-server-01',
              target_name: 'server-daily',
              last_status: 'success',
              last_backup_at: '2026-05-30T02:00:00Z',
              is_overdue: false,
              last_error_message: null,
              cron_expression: '0 2 * * *',
              schedule_enabled: true,
            },
          ],
        })
      return Promise.resolve({ data: [] })
    })
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    // Header badge: one target overdue.
    expect(wrapper.text()).toContain('1 target overdue')
    expect(wrapper.text()).toContain('Overdue')
    expect(wrapper.text()).toContain('Agent offline (last seen')

    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Retry')
    expect(retryButton).toBeTruthy()
    await retryButton!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith('/schedules/1/run', { agent_ids: [10] })
  })

  it('seeds running state from recent reports and shows Cancel backup instead of Run now', async () => {
    setupEditModeWithReport({ id: 1, status: 'started' })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const buttons = wrapper.findAll('button').map((b) => b.text())
    expect(buttons).toContain('Cancel backup')
    expect(buttons).not.toContain('Run now')
  })

  it('calls cancel API when Cancel backup is clicked', async () => {
    setupEditModeWithReport({ id: 1, status: 'pending' })
    mockApiClient.post.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const cancelBtn = wrapper.findAll('button').find((b) => b.text() === 'Cancel backup')
    await cancelBtn!.trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith('/schedules/1/cancel')
  })

  it('switches to the Backups tab from the Overview preview\'s "View all" link', async () => {
    setupEditModeWithReport({
      id: 1,
      status: 'success',
      finished_at: '2026-06-01T02:00:00Z',
      agent_id: 10,
      original_size: 100,
      duration_secs: 10,
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    expect(
      wrapper
        .findAll('.tab')
        .find((t) => t.attributes('aria-selected') === 'true')!
        .text(),
    ).toBe('Overview')

    await wrapper.find('.section-link').trigger('click')
    await flushPromises()

    expect(
      wrapper
        .findAll('.tab')
        .find((t) => t.attributes('aria-selected') === 'true')!
        .text(),
    ).toBe('Backups')
  })

  // A run in the preview is a way in, not just a status line: its archive is
  // on this schedule's own Backups tab, one click away.
  it('selects the archive of a preview run on the Backups tab', async () => {
    setupEditModeWithReport({
      id: 1,
      status: 'success',
      finished_at: '2026-06-01T02:00:00Z',
      started_at: '2026-06-01T01:50:00Z',
      agent_id: 10,
      original_size: 100,
      duration_secs: 10,
      archive_name: 'web-server-01-2026-06-01',
      error_message: null,
      warnings: [],
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const rows = wrapper.findAll('.agent-row')
    await rows[rows.length - 1].find('button.agent-row-name').trigger('click')
    await flushPromises()

    expect(
      wrapper
        .findAll('.tab')
        .find((t) => t.attributes('aria-selected') === 'true')!
        .text(),
    ).toBe('Backups')
    expect(wrapper.findComponent({ name: 'ScheduleBackupsTab' }).props('selected')).toMatchObject({
      id: 1,
    })
  })

  // A failed run wrote no archive, so this tab's browser has nothing to show
  // for it - the output lives on the host that produced the run.
  it('sends a failed preview run to its output on the host', async () => {
    setupEditModeWithReport({
      id: 7,
      status: 'failed',
      finished_at: '2026-06-01T02:00:00Z',
      started_at: '2026-06-01T01:50:00Z',
      agent_id: 10,
      original_size: 0,
      duration_secs: 5,
      archive_name: null,
      error_message: 'Repository lock could not be acquired',
      warnings: [],
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const viewError = wrapper.findAll('button').find((b) => b.text() === 'View error')
    expect(viewError).toBeDefined()
    await viewError!.trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { fullPath: string } } } })
      .$router
    expect(router.currentRoute.value.fullPath).toBe('/agents/web-server-01?tab=backups&report=7')
  })

  // The row offers the jump on any run with output, and the host it belongs
  // to is resolved at click time - so a run whose host cannot be named (an
  // agent since deleted, no hostname on the report either) has to stay put
  // rather than route to `/agents/`.
  it('stays put when a preview run names no host to open', async () => {
    setupEditModeWithReport({
      id: 8,
      agent_id: 999,
      hostname: null,
      status: 'failed',
      finished_at: '2026-06-01T02:00:00Z',
      started_at: '2026-06-01T01:50:00Z',
      original_size: 0,
      duration_secs: 5,
      archive_name: null,
      error_message: 'Repository lock could not be acquired',
      warnings: [],
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'View error')!
      .trigger('click')
    await flushPromises()

    const router = (wrapper.vm as { $router: { currentRoute: { value: { path: string } } } })
      .$router
    expect(router.currentRoute.value.path).not.toContain('/agents/')
    expect(logger.error).toHaveBeenCalledWith('cannot open a run whose host is unknown', 8)
  })

  it('reorders targets from the real Settings tab and saves the new order', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({
          data: [
            { agent_id: 10, execution_order: 0 },
            { agent_id: 11, execution_order: 1 },
          ],
        })
      if (url === '/schedules/1/sources')
        return Promise.resolve({
          data: { backup_sources: ['/data'], backup_sources_per_agent: [] },
        })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Targets')

    await wrapper.find('.order-btn[title="Move down"]').trigger('click')
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Save changes')!
      .trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({ agent_ids: [11, 10] }),
    )
  })

  it('propagates form/overrides/per-host-sources updates emitted by the real Settings tab', async () => {
    const wrapper = await createEditWrapper()
    await goToSettings(wrapper)

    const settingsTab = wrapper.findComponent({ name: 'ScheduleSettingsTab' })
    expect(settingsTab.exists()).toBe(true)

    const newForm = {
      ...(settingsTab.props('form') as Record<string, unknown>),
      name: 'Renamed via emit',
    }
    const newOverrides = {
      ...(settingsTab.props('overrides') as Record<string, unknown>),
      usePerHostExcludes: true,
    }
    await settingsTab.vm.$emit('update:form', newForm)
    await settingsTab.vm.$emit('update:overrides', newOverrides)
    await settingsTab.vm.$emit('update:perHostSources', { 10: '/custom/path' })

    expect(settingsTab.props('form')).toEqual(newForm)
    expect(settingsTab.props('overrides')).toEqual(newOverrides)
    expect(settingsTab.props('perHostSources')).toEqual({ 10: '/custom/path' })
  })

  it('propagates repo and on-failure select changes from the real Settings tab into the save payload', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({ data: [{ agent_id: mockSchedule.agent_id, execution_order: 0 }] })
      if (url === '/schedules/1/sources')
        return Promise.resolve({
          data: { backup_sources: ['/data'], backup_sources_per_agent: [] },
        })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Targets')

    const selects = wrapper.findAll('select')
    await selects[0].setValue('21')
    await selects[1].setValue('continue')

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Save changes')!
      .trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({ repo_id: 21, on_failure: 'continue' }),
    )
  })

  it('propagates the per-host-paths toggle and a per-host textarea into the save payload', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({
          data: [
            { agent_id: 10, execution_order: 0 },
            { agent_id: 11, execution_order: 1 },
          ],
        })
      if (url === '/schedules/1/sources')
        return Promise.resolve({
          data: { backup_sources: ['/data'], backup_sources_per_agent: [] },
        })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Targets')

    await wrapper.find('.toggle-switch-stub').setValue(true)
    await nextTick()
    await wrapper.find('.area-input-sm').setValue('/custom/backup/path')

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Save changes')!
      .trigger('click')
    await flushPromises()

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({
        backup_sources_per_agent: [{ agent_id: 10, paths: ['/custom/backup/path'] }],
      }),
    )
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

  it('shows only the Settings tab - there is no status yet to give Overview or Backups content', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab')
    expect(tabs).toHaveLength(1)
    expect(tabs[0].text()).toBe('Settings')
  })

  it('shows agent and repo pickers under the Targets section', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()
    await goToSection(wrapper, 'Targets')

    expect(wrapper.text()).toContain('Select agents...')
    expect(wrapper.text()).toContain('server-daily')
  })

  it('shows schedule type selector', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    expect(wrapper.text()).toContain('Schedule type')
    expect(wrapper.text()).toContain('Integrity check')
    expect(wrapper.text()).toContain('Verify (extract dry-run)')
  })

  it('shows Create schedule button', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    const createBtn = wrapper.findAll('button').find((b) => b.text() === 'Create schedule')
    expect(createBtn).toBeTruthy()
  })

  it('propagates the Schedule type select into the create payload', async () => {
    setupCreateMode()
    mockApiClient.post.mockResolvedValue({ data: { id: 5 } })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    await wrapper.find('select').setValue('check')

    await goToSection(wrapper, 'Targets')
    await wrapper.find('.multi-select-trigger').trigger('click')
    await wrapper.findAll('.multi-select-item input[type="checkbox"]')[0].trigger('change')

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Create schedule')!
      .trigger('click')
    await flushPromises()

    expect(mockApiClient.post).toHaveBeenCalledWith(
      '/schedules',
      expect.objectContaining({ schedule_type: 'check' }),
    )
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

  async function createActiveBackupWrapper(): Promise<ReturnType<typeof renderWithPlugins>> {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
      started_at: new Date().toISOString(),
    })
    await nextTick()
    return wrapper
  }

  const startedPayload = {
    hostname: 'web-server-01',
    target_name: 'server-daily',
    archive_name: null,
    schedule_id: 1,
    started_at: new Date().toISOString(),
  }

  it('BackupStarted with matching schedule_id shows the live progress card', async () => {
    const wrapper = await createEditWrapper()
    wsHandlers['BackupStarted']?.({
      hostname: 'web-server-01',
      target_name: 'server-daily',
      archive_name: 'server-daily-2026-06-26',
      schedule_id: 1,
      started_at: new Date().toISOString(),
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
    expect(wrapper.text()).toContain('Backup in progress')
  })

  it('BackupStarted with non-matching schedule_id does not activate progress card', async () => {
    const wrapper = await createEditWrapper()
    wsHandlers['BackupStarted']?.({ ...startedPayload, schedule_id: 999, archive_name: null })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupStarted with null schedule_id and matching repo name activates progress card', async () => {
    const wrapper = await createEditWrapper()
    wsHandlers['BackupStarted']?.({ ...startedPayload, schedule_id: null, archive_name: null })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(true)
  })

  it('resolves the live backup hostname to the agent display name, matching agentLabel', async () => {
    const wrapper = await createEditWrapper()
    wsHandlers['BackupStarted']?.(startedPayload)
    await nextTick()

    // mockAgents' id 10 has hostname 'web-server-01' and display_name 'Web Server' -
    // the badge must show the display name, not the raw WS hostname, so it lines up
    // with ScheduleOverviewTab's agentLabel(id)-based accent-stripe match.
    expect(wrapper.find('.live-log-card').text()).toContain('Web Server')
    const targetRow = wrapper.findAll('.agent-row').find((r) => r.text().includes('Web Server'))
    expect(targetRow!.find('.agent-row-stripe').classes()).toContain('agent-row-stripe--accent')
  })

  it('BackupCompleted with matching schedule_id hides the live progress card', async () => {
    const wrapper = await createActiveBackupWrapper()

    wsHandlers['BackupCompleted']?.({
      hostname: 'web-server-01',
      target_name: 'server-daily',
      archive_name: null,
      schedule_id: 1,
    })
    await nextTick()

    expect(wrapper.find('.live-log-card').exists()).toBe(false)
  })

  it('BackupLog with matching schedule_id and archive_progress JSON updates progress data', async () => {
    const wrapper = await renderAndStartBackup()

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
    setupEditModeWithReport({
      id: 1,
      status: 'started',
      started_at: '2026-06-27T10:00:00Z',
      agent_id: 10,
      original_size: 0,
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
    const wrapper = await renderAndStartBackup()

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
    const wrapper = await renderAndStartBackup()

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
    const wrapper = await renderAndStartBackup()

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

  it('BackupLog with non-JSON line does not render raw log output', async () => {
    const wrapper = await renderAndStartBackup()

    wsHandlers['BackupLog']?.({
      hostname: 'web-server-01',
      schedule_id: 1,
      repo_id: 20,
      line: 'Creating archive server-daily-2026-06-26...',
    })
    await nextTick()

    expect(wrapper.find('.live-log-output').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Creating archive server-daily-2026-06-26...')
  })
})

describe('ScheduleDetailView - Backups tab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  function setupBackupWithReports(mockReports: unknown[]): void {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets')
        return Promise.resolve({ data: [{ agent_id: mockSchedule.agent_id, execution_order: 0 }] })
      if (url === '/schedules/1/sources')
        return Promise.resolve({
          data: { backup_sources: ['/data'], backup_sources_per_agent: [] },
        })
      if (url === '/schedules/1/reports') return Promise.resolve({ data: mockReports })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
  }

  async function createBackupsWrapper(
    reports: unknown[],
  ): Promise<ReturnType<typeof renderWithPlugins>> {
    setupBackupWithReports(reports)
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    return wrapper
  }

  async function goToBackups(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
    await wrapper
      .findAll('.tab')
      .find((t) => t.text() === 'Backups')!
      .trigger('click')
    await flushPromises()
  }

  it('shows Backups tab button for backup-type schedule in edit mode', async () => {
    const wrapper = await createBackupsWrapper([])

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Backups')).toBe(true)
  })

  it('does NOT show Backups tab button for check-type schedule', async () => {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/2') return Promise.resolve({ data: mockCheckSchedule })
      if (url === '/schedules/2/targets')
        return Promise.resolve({
          data: [{ agent_id: mockCheckSchedule.agent_id, execution_order: 0 }],
        })
      if (url === '/schedules/2/sources')
        return Promise.resolve({ data: { backup_sources: [], backup_sources_per_agent: [] } })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '2' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Backups')).toBe(false)
  })

  it('does NOT show Backups tab button in create mode', async () => {
    setupCreateMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: 'new' } })
    await flushPromises()

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Backups')).toBe(false)
  })

  it('shows empty state when no reports have archive_name', async () => {
    const wrapper = await createBackupsWrapper([
      {
        id: 1,
        status: 'success',
        archive_name: null,
        started_at: '2026-06-01T02:00:00Z',
        original_size: 100,
        agent_id: 10,
      },
    ])

    await goToBackups(wrapper)

    expect(wrapper.text()).toContain('No backup archives found')
  })

  it('shows archive rows when reports have archive_name', async () => {
    const wrapper = await createBackupsWrapper([
      {
        id: 1,
        status: 'success',
        archive_name: 'test-archive-2026-06-02',
        started_at: '2026-06-02T02:00:00Z',
        original_size: 500,
        agent_id: 10,
        hostname: 'web-server-01',
      },
      {
        id: 2,
        status: 'success',
        archive_name: 'test-archive-2026-06-01',
        started_at: '2026-06-01T02:00:00Z',
        original_size: 400,
        agent_id: 10,
        hostname: 'web-server-01',
      },
      {
        id: 3,
        status: 'success',
        archive_name: null,
        started_at: '2026-06-01T03:00:00Z',
        original_size: 200,
        agent_id: 10,
        hostname: 'web-server-01',
      },
    ])

    await goToBackups(wrapper)

    const texts = wrapper.text()
    expect(texts).toContain('test-archive-2026-06-02')
    expect(texts).toContain('test-archive-2026-06-01')
    expect(texts.indexOf('test-archive-2026-06-02')).toBeLessThan(
      texts.indexOf('test-archive-2026-06-01'),
    )
    expect(texts).not.toContain('No backup archives found')
  })

  it('clicking an archive row selects it and shows file browser', async () => {
    const wrapper = await createBackupsWrapper([
      {
        id: 1,
        status: 'success',
        archive_name: 'test-archive-2026-06-01',
        started_at: '2026-06-01T02:00:00Z',
        original_size: 500,
        agent_id: 10,
        hostname: 'web-server-01',
      },
    ])

    await goToBackups(wrapper)

    await wrapper.find('.archive-row-select').trigger('click')
    await flushPromises()

    expect(wrapper.find('.archive-row').classes()).toContain('selected')
    expect(wrapper.find('.archive-file-browser-stub').exists()).toBe(true)
  })

  it('releases a row whose borg delete failed, once the repo queue goes idle', async () => {
    // Deleting is new on this tab. Without the repo-idle event wired through,
    // a delete that borg accepted and then failed left the archive in the list
    // with its row stuck on "Deleting" until a full page reload: the archive is
    // still present, so neither ArchiveDeleted nor the DataChanged prune ever
    // covers it.
    setupBackupWithReports([
      {
        id: 1,
        status: 'success',
        archive_name: 'test-archive-2026-06-01',
        started_at: '2026-06-01T02:00:00Z',
        original_size: 500,
        deduplicated_size: 200,
        agent_id: 10,
        hostname: 'web-server-01',
      },
    ])
    mockApiClient.delete.mockResolvedValue({
      data: { success: true, archive_name: 'test-archive-2026-06-01' },
    })

    const wrapper = renderWithPlugins(ScheduleDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await goToBackups(wrapper)

    await wrapper.find('button[title="Delete archive"]').trigger('click')
    await flushPromises()
    const confirm = document.body.querySelector<HTMLButtonElement>(
      '.modal-dialog button.btn-danger',
    )
    expect(confirm).not.toBeNull()
    confirm!.click()
    await flushPromises()

    expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

    // borg failed, so the archive is still listed - but the repository's
    // operation queue has drained, which means the delete is over either way.
    wsHandlers['RepoOpChanged']?.({ repo_id: 20, op: null })
    await flushPromises()

    expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
    expect(wrapper.find('button[title="Delete archive"]').exists()).toBe(true)
  })

  it('refetches the schedule reports when the server reports data changed', async () => {
    const wrapper = await createBackupsWrapper([
      {
        id: 1,
        status: 'success',
        archive_name: 'test-archive-2026-06-01',
        started_at: '2026-06-01T02:00:00Z',
        original_size: 500,
        agent_id: 10,
        hostname: 'web-server-01',
      },
    ])
    await goToBackups(wrapper)

    const before = mockApiClient.get.mock.calls.filter((c) =>
      String(c[0]).includes('/reports'),
    ).length

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    const after = mockApiClient.get.mock.calls.filter((c) =>
      String(c[0]).includes('/reports'),
    ).length
    // loadReports() now fetches the report list and the unbounded failed
    // count in parallel, so one refetch is two "/reports"-matching calls.
    expect(after).toBe(before + 2)
  })

  // The count backs a menu badge, not the page itself - a failure fetching
  // it during a refresh must not break the report list refresh alongside it
  // (regression: it used to sit inside the same Promise.all as the report
  // list, so a rejection there took the whole refresh down with it).
  it('logs and keeps the report list refresh working when the count fetch fails', async () => {
    const report = {
      id: 1,
      status: 'success',
      archive_name: 'test-archive-2026-06-01',
      started_at: '2026-06-01T02:00:00Z',
      original_size: 500,
      agent_id: 10,
      hostname: 'web-server-01',
    }
    const wrapper = await createBackupsWrapper([report])
    await goToBackups(wrapper)

    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1/reports/failed/count') return Promise.reject(new Error('boom'))
      if (url === '/schedules/1/reports') return Promise.resolve({ data: [report] })
      return Promise.resolve({ data: [] })
    })

    wsHandlers['DataChanged']?.({})
    await flushPromises()

    expect(logger.error).toHaveBeenCalledWith(
      'countFailedScheduleReports failed',
      expect.any(Error),
    )
    expect(wrapper.text()).toContain('test-archive-2026-06-01')
  })

  it('releases the row the server names as finished deleting', async () => {
    setupBackupWithReports([
      {
        id: 1,
        status: 'success',
        archive_name: 'test-archive-2026-06-01',
        started_at: '2026-06-01T02:00:00Z',
        original_size: 500,
        deduplicated_size: 200,
        agent_id: 10,
        hostname: 'web-server-01',
      },
    ])
    mockApiClient.delete.mockResolvedValue({
      data: { success: true, archive_name: 'test-archive-2026-06-01' },
    })

    const wrapper = renderWithPlugins(ScheduleDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await goToBackups(wrapper)

    await wrapper.find('button[title="Delete archive"]').trigger('click')
    await flushPromises()
    document.body.querySelector<HTMLButtonElement>('.modal-dialog button.btn-danger')!.click()
    await flushPromises()

    expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(true)

    // The server names exactly which archive finished, so the marker goes
    // without waiting for a refetch to notice the row is gone.
    wsHandlers['ArchiveDeleted']?.({ repo_id: 20, archive_name: 'test-archive-2026-06-01' })
    await flushPromises()

    expect(wrapper.find('button[title="Deletion in progress"]').exists()).toBe(false)
  })

  it('hides save bar on Backups tab', async () => {
    const wrapper = await createBackupsWrapper([
      {
        id: 1,
        status: 'success',
        archive_name: 'test-archive-2026-06-01',
        started_at: '2026-06-01T02:00:00Z',
        original_size: 500,
        agent_id: 10,
        hostname: 'web-server-01',
      },
    ])

    await goToBackups(wrapper)

    expect(wrapper.find('.save-bar').exists()).toBe(false)
  })

  it('resets selected archive when schedule id changes', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()

    const vm = wrapper.vm as { selectedBackupReport: { id: number; archive_name: string } | null }
    vm.selectedBackupReport = { id: 1, archive_name: 'test-archive' }
    await nextTick()
    expect(vm.selectedBackupReport).not.toBeNull()

    setupEditMode({ ...mockSchedule, id: 2 })
    await wrapper.setProps({ id: '2' })
    await flushPromises()

    expect(vm.selectedBackupReport).toBeNull()
  })
})

describe('ScheduleDetailView - header actions', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  async function openMenu(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
  }

  it('reaches the delete confirmation through the header overflow menu', async () => {
    const wrapper = await createEditWrapper()
    await openMenu(wrapper)

    const deleteItem = wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Delete schedule')
    expect(deleteItem).toBeTruthy()
    await deleteItem!.trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
    expect(wrapper.text()).toContain('This action cannot be undone.')
  })

  it('navigates to the activity log filtered by schedule from the Logs menu item', async () => {
    const wrapper = await createEditWrapper()
    await openMenu(wrapper)

    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Logs')!
      .trigger('click')
    await flushPromises()

    expect(wrapper.vm.$route.fullPath).toBe('/activity?category=backup&schedule_id=1')
  })
})

describe('ScheduleDetailView - delete confirmation', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  async function openDeleteDialog(): Promise<ReturnType<typeof renderWithPlugins>> {
    const wrapper = await createEditWrapper()
    await wrapper.find('.overflow-toggle').trigger('click')
    await wrapper
      .findAll('.overflow-menu-item')
      .find((b) => b.text().trim() === 'Delete schedule')!
      .trigger('click')
    await flushPromises()
    return wrapper
  }

  it('names what is about to be lost before deleting', async () => {
    const wrapper = await openDeleteDialog()

    expect(mockApiClient.delete).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('All associated backup reports will also be removed.')
    expect(wrapper.text()).toContain('This action cannot be undone.')
  })

  it('deletes the schedule on confirmation', async () => {
    mockApiClient.delete.mockResolvedValue({ data: {} })
    const wrapper = await openDeleteDialog()

    await wrapper
      .findAll('.modal-footer button')
      .find((b) => b.text().trim() === 'Delete schedule')!
      .trigger('click')
    await flushPromises()

    expect(mockApiClient.delete).toHaveBeenCalledWith('/schedules/1')
  })

  it('reports a failed delete and closes the dialog', async () => {
    mockApiClient.delete.mockRejectedValue(new Error('schedule running'))
    const wrapper = await openDeleteDialog()

    await wrapper
      .findAll('.modal-footer button')
      .find((b) => b.text().trim() === 'Delete schedule')!
      .trigger('click')
    await flushPromises()

    expect(wrapper.find('.state-error, .form-error, .error-banner').exists()).toBe(true)
    expect(openModals(wrapper)).toHaveLength(0)
  })

  // Deleting takes every backup report with it, so both ways out of the
  // confirmation have to leave the schedule alone.
  it.each([
    [
      'Cancel',
      async (w: ReturnType<typeof renderWithPlugins>): Promise<void> => {
        await w
          .findAll('.modal-footer button')
          .find((b) => b.text().trim() === 'Cancel')!
          .trigger('click')
        await flushPromises()
      },
    ],
    ['a dismissal', dismissModal],
  ])('backs out of the delete on %s', async (_how, close) => {
    const wrapper = await openDeleteDialog()

    await close(wrapper)

    expect(mockApiClient.delete).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })
})

// A failed run has no archive behind it, so clearing it out is safe - the
// server (not this component) is the source of truth for who may do it,
// same as the pre-existing Delete-schedule flow above.
describe('ScheduleDetailView - clean up failed backups', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  async function openMenu(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
    await wrapper.find('.overflow-toggle').trigger('click')
    await flushPromises()
  }

  async function createAdminWrapperWithFailedReport(): Promise<
    ReturnType<typeof renderWithPlugins>
  > {
    setupEditModeWithReport({ id: 1, status: 'failed', started_at: '2026-06-01T02:00:00Z' })
    const wrapper = renderWithPlugins(ScheduleDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    return wrapper
  }

  it('is omitted when nothing has failed', async () => {
    setupEditMode()
    const wrapper = renderWithPlugins(ScheduleDetailView, {
      props: { id: '1' },
      storeState: { auth: { user: { role: 'admin' } } },
    })
    await flushPromises()
    await openMenu(wrapper)

    expect(
      wrapper.findAll('.overflow-menu-item').some((i) => i.text().startsWith('Clean up failed')),
    ).toBe(false)
  })

  it('reaches the confirmation through the header overflow menu', async () => {
    const wrapper = await createAdminWrapperWithFailedReport()
    await openMenu(wrapper)

    const cleanItem = wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')
    expect(cleanItem).toBeTruthy()
    await cleanItem!.trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
    expect(wrapper.text()).toContain('This action cannot be undone.')
  })

  it('closes the dialog without deleting when Cancel is clicked', async () => {
    const wrapper = await createAdminWrapperWithFailedReport()
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

    expect(mockApiClient.delete).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('closes the dialog without deleting on dismissal', async () => {
    const wrapper = await createAdminWrapperWithFailedReport()
    await openMenu(wrapper)
    await wrapper
      .findAll('.overflow-menu-item')
      .find((i) => i.text() === 'Clean up failed backups (1)')!
      .trigger('click')
    await flushPromises()

    await dismissModal(wrapper)

    expect(mockApiClient.delete).not.toHaveBeenCalled()
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('deletes the failed reports on confirmation', async () => {
    mockApiClient.delete.mockResolvedValue({ data: { deleted: 1 } })
    const wrapper = await createAdminWrapperWithFailedReport()
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

    expect(mockApiClient.delete).toHaveBeenCalledWith('/schedules/1/reports/failed')
    expect(openModals(wrapper)).toHaveLength(0)
  })

  it('reports a failure and leaves the dialog open', async () => {
    mockApiClient.delete.mockRejectedValue(new Error('locked'))
    const wrapper = await createAdminWrapperWithFailedReport()
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

    expect(openModals(wrapper)).toHaveLength(1)
  })
})

describe('ScheduleDetailView - per-agent overrides', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  const TWO_TARGETS = [
    { agent_id: 10, execution_order: 0 },
    { agent_id: 11, execution_order: 1 },
  ]

  function setupWithSources(sources: Record<string, unknown>): void {
    mockApiClient.get.mockImplementation((url: string) => {
      if (url === '/schedules/1') return Promise.resolve({ data: mockSchedule })
      if (url === '/schedules/1/targets') return Promise.resolve({ data: TWO_TARGETS })
      if (url === '/schedules/1/sources') return Promise.resolve({ data: sources })
      if (url === '/agents') return Promise.resolve({ data: mockAgents })
      if (url === '/repos') return Promise.resolve({ data: mockRepos })
      return Promise.resolve({ data: [] })
    })
  }

  async function renderAdvanced(
    sources: Record<string, unknown>,
  ): Promise<ReturnType<typeof renderWithPlugins>> {
    setupWithSources(sources)
    mockApiClient.put.mockResolvedValue({ data: mockSchedule })
    const wrapper = renderWithPlugins(ScheduleDetailView, { props: { id: '1' } })
    await flushPromises()
    await goToSettings(wrapper)
    await goToSection(wrapper, 'Advanced')
    return wrapper
  }

  async function save(wrapper: ReturnType<typeof renderWithPlugins>): Promise<void> {
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Save changes')!
      .trigger('click')
    await flushPromises()
  }

  // A schedule saved with per-agent excludes has to come back in per-agent
  // mode: reopening it in shared mode would silently flatten every host's
  // list into one on the next save.
  it('reopens in per-agent mode and sends the excludes back per agent', async () => {
    const wrapper = await renderAdvanced({
      backup_sources: [],
      backup_sources_per_agent: [
        { agent_id: 10, paths: ['/srv'] },
        { agent_id: 11, paths: ['/var'] },
      ],
      exclude_patterns_per_agent: [
        { agent_id: 10, raw_text: '*.cache' },
        { agent_id: 11, raw_text: '*.tmp' },
      ],
    })

    await save(wrapper)

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({
        exclude_patterns_raw: '',
        exclude_patterns_per_agent: [
          { agent_id: 10, raw_text: '*.cache' },
          { agent_id: 11, raw_text: '*.tmp' },
        ],
        backup_sources_per_agent: [
          { agent_id: 10, paths: ['/srv'] },
          { agent_id: 11, paths: ['/var'] },
        ],
      }),
    )
  })

  it('reopens in per-agent mode and sends the file change patterns back per agent', async () => {
    const wrapper = await renderAdvanced({
      backup_sources: ['/data'],
      backup_sources_per_agent: [],
      file_change_patterns_per_agent: [
        { agent_id: 10, raw_text: '/data/wal/** ignore' },
        { agent_id: 11, raw_text: '/var/log/** warn' },
      ],
    })

    await save(wrapper)

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({
        file_change_patterns_raw: '',
        file_change_patterns_per_agent: [
          { agent_id: 10, raw_text: '/data/wal/** ignore' },
          { agent_id: 11, raw_text: '/var/log/** warn' },
        ],
      }),
    )
  })

  it('reopens in per-agent mode and sends both hook command lists back per agent', async () => {
    const wrapper = await renderAdvanced({
      backup_sources: ['/data'],
      backup_sources_per_agent: [],
      commands_per_agent: [
        {
          agent_id: 10,
          pre_backup_commands: [hookCommand('stop-app')],
          post_backup_commands: [hookCommand('start-app')],
        },
        { agent_id: 11, pre_backup_commands: [], post_backup_commands: [] },
      ],
    })

    await save(wrapper)

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({
        commands_per_agent: [
          {
            agent_id: 10,
            pre_backup_commands: [hookCommand('stop-app')],
            post_backup_commands: [hookCommand('start-app')],
          },
          { agent_id: 11, pre_backup_commands: [], post_backup_commands: [] },
        ],
      }),
    )
  })

  // A CommandListEditor entry can be left blank (e.g. after "+ Add command"
  // with nothing typed yet); saving must drop those rather than sending an
  // empty-string command through to the agent.
  it('drops blank command entries but keeps real ones when saving', async () => {
    const wrapper = await renderAdvanced({
      backup_sources: ['/data'],
      backup_sources_per_agent: [],
    })

    const editors = wrapper.findAllComponents({ name: 'CommandListEditor' })
    await editors[0].vm.$emit('update:modelValue', [
      hookCommand('docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql'),
      hookCommand(''),
      hookCommand('   '),
    ])
    await editors[1].vm.$emit('update:modelValue', [
      hookCommand(''),
      hookCommand('systemctl start app'),
    ])

    await save(wrapper)

    expect(mockApiClient.put).toHaveBeenCalledWith(
      '/schedules/1',
      expect.objectContaining({
        pre_backup_commands: [
          hookCommand('docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql'),
        ],
        post_backup_commands: [hookCommand('systemctl start app')],
      }),
    )
  })

  // The shared fields stay in charge when the server sent no per-agent rows,
  // so an empty list must not flip the schedule into per-agent mode.
  it('stays in shared mode when no per-agent rows come back', async () => {
    const wrapper = await renderAdvanced({
      backup_sources: ['/data'],
      backup_sources_per_agent: [],
      exclude_patterns_per_agent: [],
      file_change_patterns_per_agent: [],
      commands_per_agent: [],
    })

    await save(wrapper)

    const payload = mockApiClient.put.mock.calls[0][1] as Record<string, unknown>
    expect(payload.exclude_patterns_per_agent).toBeUndefined()
    expect(payload.file_change_patterns_per_agent).toBeUndefined()
    expect(payload.commands_per_agent).toBeUndefined()
  })
})
