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
        findings: [],
        summary: {
          protected_hosts: 0,
          eligible_hosts: 0,
          needs_attention: 0,
          running_operations: 0,
          total_storage_bytes: 0,
        },
        protection: {
          protected_hosts: 0,
          eligible_hosts: 0,
          protected_agent_links: [],
          unassigned_agents: [],
          never_succeeded_targets: 0,
          never_succeeded_agents: [],
          disabled_only_agents: [],
        },
        repository_capacity: [],
        upcoming_schedules: [],
        running_operations: [],
      },
    })
  }
  return Promise.resolve({ data: [] })
}

vi.mock('vue-chartjs', () => {
  const Line = { template: '<canvas />' }
  return { Line }
})

vi.mock('chart.js', () => {
  const Chart = { register: vi.fn() }
  const CategoryScale = {}
  const LinearScale = {}
  const PointElement = {}
  const LineElement = {}
  const Title = {}
  const Tooltip = {}
  const Legend = {}
  const Filler = {}
  return {
    Chart,
    CategoryScale,
    LinearScale,
    PointElement,
    LineElement,
    Title,
    Tooltip,
    Legend,
    Filler,
  }
})

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn() },
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

/** Overview response with a single finding for tests that verify findings rendering. */
function overviewWithFindings() {
  return {
    summary: {
      protected_hosts: 0,
      eligible_hosts: 0,
      needs_attention: 1,
      running_operations: 0,
      total_storage_bytes: 0,
    },
    findings: [
      {
        id: 'f1',
        kind: 'backup_failed',
        severity: 'critical',
        reason: 'Backup failed',
        destination: { kind: 'host', hostname: 'web-01' },
      },
    ],
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
  }
}

vi.mocked(apiClient.get).mockImplementation(defaultApiHandler)

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

describe('DashboardView attention panel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.mocked(apiClient.get).mockImplementation(defaultApiHandler)
  })

  it('hides NeedsAttention when findings are empty', async () => {
    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    expect(wrapper.find('#needs-attention').exists()).toBe(false)
  })

  it('shows NeedsAttention when findings exist', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({ data: overviewWithFindings() })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    expect(wrapper.find('#needs-attention').exists()).toBe(true)
  })

  it('applies attention-sidebar-wide class when findings are empty', async () => {
    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    const sidebar = wrapper.find('.attention-sidebar')
    expect(sidebar.classes()).toContain('attention-sidebar-wide')
  })

  it('does not apply attention-sidebar-wide class when findings exist', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({ data: overviewWithFindings() })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    const sidebar = wrapper.find('.attention-sidebar')
    expect(sidebar.classes()).not.toContain('attention-sidebar-wide')
  })

  it('re-fetches overview when findings are dismissed', async () => {
    const getSpy = vi.mocked(apiClient.get)
    getSpy.mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({ data: overviewWithFindings() })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    // NeedsAttention should be visible when findings exist
    expect(wrapper.find('#needs-attention').exists()).toBe(true)

    // Emit dismissed from the parent component's scope via the NeedsAttention component
    // We find it by component name and emit on its wrapper
    const needsAttWrapper = wrapper.findComponent({ name: 'NeedsAttention' })
    if (needsAttWrapper.exists()) {
      needsAttWrapper.vm.$emit('dismissed')
      await flushPromises()
    }

    // The fetchOverview call should have been made again (overview endpoint called at least twice)
    const overviewCalls = getSpy.mock.calls.filter(([url]) => url === '/stats/dashboard-overview')
    expect(overviewCalls.length).toBeGreaterThanOrEqual(2)
  })

  it('renders fallback em-dash when summary lacks next_backup_at', async () => {
    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    // The default overview response has no next_backup_at, so the fallback should appear
    const dashPlaceholder = wrapper.text()
    expect(dashPlaceholder).toContain('\u2014')
  })

  it('applies attention-row-full class when findings are empty', async () => {
    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    const row = wrapper.find('.attention-row')
    expect(row.classes()).toContain('attention-row-full')
  })

  it('removes attention-row-full class when findings exist', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({ data: overviewWithFindings() })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    const row = wrapper.find('.attention-row')
    expect(row.classes()).not.toContain('attention-row-full')
  })

  it('hides NeedsAttention after dismiss when fetchOverview returns empty findings', async () => {
    const getSpy = vi.mocked(apiClient.get)
    getSpy.mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({ data: overviewWithFindings() })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    expect(wrapper.find('#needs-attention').exists()).toBe(true)

    // On next fetchOverview, return findings with no results
    getSpy.mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: { ...overviewWithFindings(), findings: [] },
        })
      }
      return defaultApiHandler(url)
    })

    const needsAttWrapper = wrapper.findComponent({ name: 'NeedsAttention' })
    expect(needsAttWrapper.exists()).toBe(true)
    needsAttWrapper.vm.$emit('dismissed')
    await flushPromises()

    // After dismiss and fetchOverview with empty findings, NeedsAttention should hide
    expect(wrapper.find('#needs-attention').exists()).toBe(false)

    // The attention row should now be full width
    const row = wrapper.find('.attention-row')
    expect(row.classes()).toContain('attention-row-full')
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

  it('hydrates active backups from running operations after reload', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/stats/dashboard-overview') {
        return Promise.resolve({
          data: {
            repository_capacity: [],
            upcoming_schedules: [],
            findings: [],
            summary: {
              protected_hosts: 0,
              eligible_hosts: 0,
              needs_attention: 0,
              running_operations: 1,
              total_storage_bytes: 0,
            },
            protection: {
              protected_hosts: 0,
              eligible_hosts: 0,
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_targets: 0,
              never_succeeded_agents: [],
              disabled_only_agents: [],
            },
            running_operations: [
              {
                report_id: 11,
                status: 'running',
                hostname: 'web-server-01',
                schedule_id: 7,
                schedule_name: 'daily-web',
                repo_id: 3,
                repo_name: 'server-daily',
                started_at: '2026-06-01T10:00:00Z',
                destination: { kind: 'schedule', schedule_id: 7 },
              },
            ],
          },
        })
      }
      return defaultApiHandler(url)
    })

    const wrapper = renderWithPlugins(DashboardView)
    await flushPromises()

    expect(wrapper.text()).toContain('Backups In Progress')
    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('server-daily')
    expect(wrapper.text()).toContain('Active')
  })
})
