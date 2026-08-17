// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import TrendsChart from './TrendsChart.vue'
import { apiClient } from '../api/client'

// Records each <Line> stub's `options` prop so chart.js callbacks (tooltip
// labels, axis ticks) - never invoked by the stub itself - can be exercised
// directly, the same way chart.js would call them at render time.
const { capturedOptions } = vi.hoisted(() => ({ capturedOptions: [] as unknown[] }))

vi.mock('vue-chartjs', () => ({
  Line: {
    props: ['data', 'options'],
    template: '<canvas data-testid="line-chart" />',
    created(): void {
      capturedOptions.push((this as unknown as { options: unknown }).options)
    },
  },
}))

// jscpd:ignore-start -- test setup boilerplate (vi.mock factories cannot reference module-scoped helpers)
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
// jscpd:ignore-end

describe('TrendsChart', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    capturedOptions.length = 0
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

  // The range buttons drive the query, so a click has to reach the fetch -
  // a control that only repaints itself looks like it worked and does not.
  it('refetches the trend over the chosen range', async () => {
    const mockGet = vi.mocked(apiClient.get)
    mockGet.mockResolvedValue({ data: [] } as never)
    const wrapper = renderWithPlugins(TrendsChart, { props: { repos: [] } })
    await flushPromises()
    expect(mockGet).toHaveBeenCalledWith(expect.stringContaining('days=30'))

    await wrapper
      .findAll('.segmented-option')
      .find((b) => b.text() === '1y')!
      .trigger('click')
    await flushPromises()

    expect(mockGet).toHaveBeenLastCalledWith(expect.stringContaining('days=365'))
  })

  it('displays charts when trend data is available', async () => {
    const mockGet = vi.mocked(apiClient.get)
    mockGet.mockResolvedValue({
      data: [
        {
          date: '2026-05-01',
          original_size: 2_000_000_000,
          compressed_size: 1_500_000_000,
          deduplicated_size: 1_073_741_824,
          dedup_ratio: 50,
          file_count: 100,
          duration_seconds: 60,
        },
        {
          date: '2026-05-02',
          original_size: 4_000_000_000,
          compressed_size: 3_000_000_000,
          deduplicated_size: 2_147_483_648,
          dedup_ratio: 55,
          file_count: 120,
          duration_seconds: 70,
        },
      ],
    } as never)
    const wrapper = renderWithPlugins(TrendsChart, {
      props: { repos: [{ id: 1, name: 'repo-alpha' }] },
    })
    await flushPromises()

    expect(wrapper.findAll('[data-testid="line-chart"]')).toHaveLength(3)

    // Third chart is the Dedup Ratio one; format its tooltip/tick callbacks
    // the way chart.js would when actually rendering the chart.
    const dedupRatioOptions = capturedOptions[2] as {
      plugins: { tooltip: { callbacks: { label: (ctx: { parsed: { y: number } }) => string } } }
      scales: { y: { ticks: { callback: (v: number) => string } } }
    }
    expect(dedupRatioOptions.plugins.tooltip.callbacks.label({ parsed: { y: 52.34 } })).toBe(
      '52.3%',
    )
    expect(dedupRatioOptions.scales.y.ticks.callback(52.34)).toBe('52%')
  })

  it('refetches the trend for the chosen repo', async () => {
    const mockGet = vi.mocked(apiClient.get)
    mockGet.mockResolvedValue({ data: [] } as never)
    const wrapper = renderWithPlugins(TrendsChart, {
      props: { repos: [{ id: 4, name: 'repo-beta' }] },
    })
    await flushPromises()

    await wrapper.find('select').setValue('4')
    await flushPromises()

    expect(mockGet).toHaveBeenLastCalledWith(expect.stringContaining('repo_id=4'))
  })
})
