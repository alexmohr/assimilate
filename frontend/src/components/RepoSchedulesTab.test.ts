// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import RepoSchedulesTab from './RepoSchedulesTab.vue'
import { apiClient } from '../api/client'
import type * as VueRouter from 'vue-router'

type VueRouterModule = typeof VueRouter

const toastSuccess = vi.fn()
const routerPush = vi.fn()

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn() },
}))

vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: vi.fn() }),
}))

vi.mock('vue-router', async (importOriginal) => ({
  ...(await importOriginal<VueRouterModule>()),
  useRouter: () => ({ push: routerPush }),
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback?: string): string => fallback ?? 'Unknown error',
}))

const SCHEDULE = {
  id: 7,
  name: 'nightly',
  cron_expression: '0 2 * * *',
  enabled: true,
  schedule_type: 'backup',
  target_hostnames: ['web-01', 'web-02'],
  next_run_at: '2026-02-01T02:00:00Z',
  last_run_at: '2026-01-31T02:00:00Z',
}

function mockList(schedules: unknown[]): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url.endsWith('/schedules')) return Promise.resolve({ data: schedules })
    return Promise.resolve({ data: [] })
  })
}

describe('RepoSchedulesTab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('loads the schedules for its repository on mount', async () => {
    mockList([SCHEDULE])
    renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith('/repos/3/schedules')
    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith('/stats/health', { timeout: undefined })
  })

  it('renders a row per schedule, naming its agents', async () => {
    mockList([SCHEDULE, { ...SCHEDULE, id: 8, name: 'weekly', target_hostnames: ['db-01'] }])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    const rows = wrapper.findAll('[data-schedule-id]')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.find('.sched-title').text()).toBe('nightly')
    expect(rows[0]!.find('.sched-sub').text()).toContain('2 agents')
    // A single agent is named rather than counted.
    expect(rows[1]!.find('.sched-sub').text()).toContain('db-01')
  })

  it('shows an empty state when the repository has no schedules', async () => {
    mockList([])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.find('.empty-title').text()).toBe('No schedules yet')
    expect(wrapper.findAll('[data-schedule-id]')).toHaveLength(0)
  })

  it('surfaces a load failure instead of an empty list', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    expect(wrapper.find('.error-banner').text()).toBe('Failed to load schedules.')
    expect(wrapper.find('.empty-state').exists()).toBe(false)
  })

  it('marks a disabled schedule, greys its dot and offers no Run now', async () => {
    mockList([{ ...SCHEDULE, enabled: false }])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    const row = wrapper.find('[data-schedule-id="7"]')
    expect(row.find('.sched-next').text()).toBe('Disabled')
    expect(row.find('.sched-dot').classes()).toContain('sched-dot--idle')
    expect(row.find('button[aria-label="Run nightly now"]').attributes('disabled')).toBeDefined()
  })

  it('runs a schedule now and disables the button while it is in flight', async () => {
    mockList([SCHEDULE])
    let resolveRun: (() => void) | undefined
    vi.mocked(apiClient.post).mockReturnValue(
      new Promise((resolve) => {
        resolveRun = (): void => resolve({ data: {} })
      }),
    )
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    const runBtn = wrapper.find('button[aria-label="Run nightly now"]')
    expect(runBtn.exists()).toBe(true)
    expect(runBtn.attributes('disabled')).toBeUndefined()
    await runBtn.trigger('click')

    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/schedules/7/run', {})
    expect(
      wrapper.find('button[aria-label="Run nightly now"]').attributes('disabled'),
    ).toBeDefined()
    // Starting a run does not open the schedule.
    expect(routerPush).not.toHaveBeenCalled()

    resolveRun?.()
    await flushPromises()
    expect(
      wrapper.find('button[aria-label="Run nightly now"]').attributes('disabled'),
    ).toBeUndefined()
  })

  it('reloads when the repository changes', async () => {
    mockList([SCHEDULE])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await wrapper.setProps({ repoId: 9 })
    await flushPromises()

    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith('/repos/9/schedules')
  })

  // One case per ScheduleType arm: a type added to the union without a label
  // renders blank in the run-started toast, which is easy to miss.
  it.each([
    ['backup', 'Backup'],
    ['check', 'Integrity check'],
    ['verify', 'Verify (extract dry-run)'],
  ])('names a %s schedule when it is started', async (type, label) => {
    mockList([{ ...SCHEDULE, schedule_type: type }])
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} } as never)

    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    await wrapper.find('button[aria-label="Run nightly now"]').trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/schedules/7/run', {})
    expect(toastSuccess).toHaveBeenCalledWith(`${label} started.`)
  })

  it('opens the schedule when its row is selected, by click or Enter', async () => {
    mockList([SCHEDULE])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    const row = wrapper.find('[data-schedule-id="7"]')
    expect(row.attributes('role')).toBe('link')
    await row.trigger('click')
    await flushPromises()
    expect(routerPush).toHaveBeenCalledWith('/schedules/7')

    routerPush.mockClear()
    await row.trigger('keydown', { key: 'Enter' })
    expect(routerPush).toHaveBeenCalledWith('/schedules/7')
  })
})
