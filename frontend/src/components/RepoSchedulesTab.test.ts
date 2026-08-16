// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import RepoSchedulesTab from './RepoSchedulesTab.vue'
import { apiClient } from '../api/client'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn() },
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
    expect(vi.mocked(apiClient.get)).toHaveBeenCalledWith('/stats/health')
  })

  it('renders a card per schedule', async () => {
    mockList([SCHEDULE])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    expect(wrapper.findAll('.schedule-card')).toHaveLength(1)
    expect(wrapper.text()).toContain('nightly')
    expect(wrapper.text()).toContain('2 agents')
    expect(wrapper.find('.badge--neutral').text()).toBe('Backup')
  })

  it('shows an empty state when the repository has no schedules', async () => {
    mockList([])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.find('.empty-title').text()).toBe('No schedules yet')
    expect(wrapper.findAll('.schedule-card')).toHaveLength(0)
  })

  it('surfaces a load failure instead of an empty list', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    expect(wrapper.find('.state-error').text()).toBe('Failed to load schedules.')
    expect(wrapper.find('.empty-state').exists()).toBe(false)
  })

  it('marks a disabled schedule and tints its card', async () => {
    mockList([{ ...SCHEDULE, enabled: false }])
    const wrapper = renderWithPlugins(RepoSchedulesTab, { props: { repoId: 3 } })
    await flushPromises()

    const card = wrapper.find('.schedule-card')
    expect(card.classes()).toContain('schedule-card-notable')
    expect(wrapper.text()).toContain('Disabled')
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

    const runBtn = wrapper.findAll('button').find((b) => b.text() === 'Run')
    expect(runBtn).toBeDefined()
    await runBtn!.trigger('click')

    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/schedules/7/run', {})
    expect(wrapper.findAll('button')[0].attributes('disabled')).toBeDefined()

    resolveRun?.()
    await flushPromises()
    expect(wrapper.findAll('button').find((b) => b.text() === 'Run')).toBeDefined()
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
})
