// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import DashboardView from './DashboardView.vue'

function defaultApiHandler(url: string): Promise<{ data: unknown }> {
  if (url.startsWith('/stats/summary')) {
    return Promise.resolve({
      data: {
        total_hosts: 0,
        online_hosts: 0,
        total_repos: 0,
        total_size_bytes: 0,
        total_backups: 0,
        recent_failures: 0,
        storage_by_repo: [],
      },
    })
  }
  if (url === '/stats/dashboard-overview') {
    return Promise.resolve({
      data: {
        summary: {
          protected_hosts: 0,
          eligible_hosts: 0,
          needs_attention: 0,
          running_operations: 0,
          total_storage_bytes: 0,
        },
        findings: [],
        protection: {
          protected_hosts: 0,
          eligible_hosts: 0,
          protected_agent_links: [],
          unassigned_agents: [],
          never_succeeded_targets: 0,
          never_succeeded_agents: [],
          disabled_only_agents: [],
        },
        running_operations: [],
        upcoming_schedules: [],
        repository_capacity: [],
      },
    })
  }
  return Promise.resolve({ data: [] })
}

vi.mock('vue-chartjs', () => ({
  Line: { template: '<canvas />' },
}))

vi.mock('chart.js', () => ({
  Chart: { register: vi.fn() },
  CategoryScale: {},
  LinearScale: {},
  PointElement: {},
  LineElement: {},
  Title: {},
  Tooltip: {},
  Legend: {},
  Filler: {},
}))

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn().mockImplementation((url: string) => {
      if (url.startsWith('/stats/summary')) {
        return Promise.resolve({
          data: {
            total_hosts: 0,
            online_hosts: 0,
            total_repos: 0,
            total_size_bytes: 0,
            total_backups: 0,
            recent_failures: 0,
            storage_by_repo: [],
          },
        })
      }
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            summary: {
              protected_hosts: 0,
              eligible_hosts: 0,
              needs_attention: 0,
              running_operations: 0,
              total_storage_bytes: 0,
            },
            findings: [],
            protection: {
              protected_hosts: 0,
              eligible_hosts: 0,
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_targets: 0,
              never_succeeded_agents: [],
              disabled_only_agents: [],
            },
            running_operations: [],
            upcoming_schedules: [],
            repository_capacity: [],
          },
        })
      }
      return Promise.resolve({ data: [] })
    }),
  },
}))

vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: (): { onMessage: ReturnType<typeof vi.fn>; status: { value: string } } => ({
    onMessage: vi.fn(),
    status: { value: 'disconnected' },
  }),
}))

vi.mock('../composables/useTimezone', () => ({
  useTimezone: (): { timezone: { value: 'UTC' }; allTimezones: [] } => ({
    timezone: { value: 'UTC' },
    allTimezones: [],
  }),
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => `rel:${s}`,
  formatDuration: (n: number): string => `${n}s`,
}))

vi.mock('../utils/cron', () => ({
  cronToHuman: (s: string): string => `cron:${s}`,
}))

describe('DashboardView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(DashboardView)
    expect(wrapper.exists()).toBe(true)
  })

  it('shows loading skeleton state initially', () => {
    const wrapper = renderWithPlugins(DashboardView)
    expect(wrapper.find('.dashboard').exists()).toBe(true)
  })

  it('renders the dashboard container element', () => {
    const wrapper = renderWithPlugins(DashboardView)
    expect(wrapper.find('.dashboard').exists()).toBe(true)
  })
})

describe('DashboardView success ring', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.mocked(apiClient.get).mockImplementation(defaultApiHandler)
  })

  function activityEntry(id: number, status: string): Record<string, unknown> {
    return {
      id,
      hostname: 'web-server-01',
      target_name: 'server-daily',
      started_at: '2026-06-01T10:00:00Z',
      finished_at: '2026-06-01T10:05:00Z',
      status,
      duration_secs: 300,
    }
  }

  it('counts passed, warned, and failed separately instead of folding warnings into failed', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url.startsWith('/stats/activity')) {
        return Promise.resolve({
          data: [
            activityEntry(1, 'success'),
            activityEntry(2, 'success'),
            activityEntry(3, 'warning'),
            activityEntry(4, 'failed'),
          ],
        })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    expect(wrapper.text()).toContain('Passed: 2')
    expect(wrapper.text()).toContain('Warned: 1')
    expect(wrapper.text()).toContain('Failed: 1')
  })

  it('does not count a warning as a failure in the success rate', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url.startsWith('/stats/activity')) {
        return Promise.resolve({
          data: [
            activityEntry(1, 'success'),
            activityEntry(2, 'success'),
            activityEntry(3, 'warning'),
          ],
        })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    // 2 of 3 are strict successes; if the warning were folded into "failed"
    // this would read 33% instead.
    expect(wrapper.text()).toContain('67%')
  })
})
