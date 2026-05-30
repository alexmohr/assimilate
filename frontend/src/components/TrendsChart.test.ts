// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import TrendsChart from './TrendsChart.vue'

vi.mock('vue-chartjs', () => ({
  Line: { template: '<canvas data-testid="line-chart" />' },
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
    get: vi.fn().mockResolvedValue({ data: [] }),
  },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => s,
  formatDuration: (n: number): string => `${n}s`,
}))

describe('TrendsChart', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(TrendsChart, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows loading state initially', () => {
    const wrapper = renderWithPlugins(TrendsChart, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Loading trends')
  })

  it('renders repo options in select', () => {
    const wrapper = renderWithPlugins(TrendsChart, {
      props: {
        repos: [
          { id: 1, name: 'daily-backups' },
          { id: 2, name: 'weekly-archive' },
        ],
      },
    })
    expect(wrapper.text()).toContain('daily-backups')
    expect(wrapper.text()).toContain('weekly-archive')
  })

  it('renders the panel title', () => {
    const wrapper = renderWithPlugins(TrendsChart, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Backup Size Trends')
  })

  it('renders day range toggle buttons', () => {
    const wrapper = renderWithPlugins(TrendsChart, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('30d')
    expect(wrapper.text()).toContain('90d')
    expect(wrapper.text()).toContain('1y')
  })
})
