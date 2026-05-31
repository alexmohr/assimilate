// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import BackupStatsWidget from './BackupStatsWidget.vue'
import { apiClient } from '../api/client'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
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

describe('BackupStatsWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('displays the success rate percentage', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          id: 1,
          hostname: 'h1',
          target_name: 't1',
          started_at: '',
          finished_at: '',
          status: 'success',
          duration_secs: 10,
        },
        {
          id: 2,
          hostname: 'h2',
          target_name: 't2',
          started_at: '',
          finished_at: '',
          status: 'success',
          duration_secs: 10,
        },
        {
          id: 3,
          hostname: 'h3',
          target_name: 't3',
          started_at: '',
          finished_at: '',
          status: 'failed',
          duration_secs: 10,
        },
      ],
    })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('67%')
  })

  it('displays failed count', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          id: 1,
          hostname: 'h1',
          target_name: 't1',
          started_at: '',
          finished_at: '',
          status: 'success',
          duration_secs: 10,
        },
        {
          id: 2,
          hostname: 'h2',
          target_name: 't2',
          started_at: '',
          finished_at: '',
          status: 'failed',
          duration_secs: 10,
        },
        {
          id: 3,
          hostname: 'h3',
          target_name: 't3',
          started_at: '',
          finished_at: '',
          status: 'failed',
          duration_secs: 10,
        },
      ],
    })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('2')
  })

  it('shows 0% when no backups have run', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('0%')
  })
})
