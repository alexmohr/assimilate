// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import StorageTrendWidget from './StorageTrendWidget.vue'
import { apiClient } from '../api/client'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

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

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => s,
  formatDuration: (n: number): string => `${n}s`,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

const mockGet = vi.mocked(apiClient.get)

describe('StorageTrendWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows empty state when no storage data', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Not enough data.')
  })

  it('displays chart when data is available', async () => {
    mockGet.mockResolvedValue({
      data: [
        { date: '2026-05-01', original_size: 2_000_000_000, compressed_size: 1_500_000_000, deduplicated_size: 1_073_741_824 },
        { date: '2026-05-02', original_size: 4_000_000_000, compressed_size: 3_000_000_000, deduplicated_size: 2_147_483_648 },
      ],
    })
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { repos: [{ id: 1, name: 'repo-alpha' }] },
    })
    await flushPromises()
    expect(wrapper.find('canvas').exists()).toBe(true)
  })
})
