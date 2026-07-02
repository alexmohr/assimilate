// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import RecentActivityWidget from './RecentActivityWidget.vue'
import { apiClient } from '../api/client'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => `rel:${s}`,
  formatDuration: (n: number): string => `${n}s`,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

const mockGet = vi.mocked(apiClient.get)

describe('RecentActivityWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(RecentActivityWidget)
    expect(wrapper.exists()).toBe(true)
  })

  it('shows empty state when no activity', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(RecentActivityWidget)
    await flushPromises()
    expect(wrapper.text()).toContain('No recent activity.')
  })

  it('displays hostname and target for each activity entry', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          id: 1,
          hostname: 'web-server-01',
          target_name: 'daily',
          started_at: '2026-05-31T03:00:00Z',
          finished_at: '2026-05-31T03:05:00Z',
          status: 'success',
          duration_secs: 300,
          repo_id: 1,
          archive_name: null,
          error_message: null,
        },
      ],
    })
    const wrapper = renderWithPlugins(RecentActivityWidget)
    await flushPromises()
    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('daily')
  })

  it('displays duration for each activity entry', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          id: 2,
          hostname: 'db-server-01',
          target_name: 'db-backup',
          started_at: '2026-05-31T01:00:00Z',
          finished_at: '2026-05-31T01:10:00Z',
          status: 'success',
          duration_secs: 600,
          repo_id: 1,
          archive_name: null,
          error_message: null,
        },
      ],
    })
    const wrapper = renderWithPlugins(RecentActivityWidget)
    await flushPromises()
    expect(wrapper.text()).toContain('600s')
  })

  it('renders multiple entries', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          id: 1,
          hostname: 'host-a',
          target_name: 'repo-a',
          started_at: '2026-05-31T02:00:00Z',
          finished_at: '2026-05-31T02:01:00Z',
          status: 'success',
          duration_secs: 60,
          repo_id: 1,
          archive_name: null,
          error_message: null,
        },
        {
          id: 2,
          hostname: 'host-b',
          target_name: 'repo-b',
          started_at: '2026-05-31T01:00:00Z',
          finished_at: '2026-05-31T01:01:00Z',
          status: 'failed',
          duration_secs: 45,
          repo_id: null,
          archive_name: null,
          error_message: 'something went wrong',
        },
      ],
    })
    const wrapper = renderWithPlugins(RecentActivityWidget)
    await flushPromises()
    expect(wrapper.text()).toContain('host-a')
    expect(wrapper.text()).toContain('host-b')
  })

  it('renders error text in warning color for warning status entries', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          id: 1,
          hostname: 'web-01',
          target_name: 'nightly',
          started_at: '2026-05-31T04:00:00Z',
          finished_at: '2026-05-31T04:05:00Z',
          status: 'warning',
          duration_secs: 320,
          repo_id: null,
          archive_name: null,
          error_message: 'low disk space on /var',
        },
      ],
    })
    const wrapper = renderWithPlugins(RecentActivityWidget)
    await flushPromises()
    await wrapper.find('.activity-item-clickable').trigger('click')
    const pre = wrapper.find('pre')
    expect(pre.exists()).toBe(true)
    expect(pre.attributes('style')).toContain('var(--warning)')
  })
})
