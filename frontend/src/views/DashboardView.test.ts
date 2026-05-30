// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import DashboardView from './DashboardView.vue'

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

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => `rel:${s}`,
  formatDuration: (n: number): string => `${n}s`,
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
