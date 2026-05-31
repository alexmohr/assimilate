// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import NextScheduledWidget from './NextScheduledWidget.vue'
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

describe('NextScheduledWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-05-31T00:00:00Z'))
  })

  it('renders without throwing', () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(NextScheduledWidget)
    expect(wrapper.exists()).toBe(true)
  })

  it('shows no upcoming message when calendar is empty', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(NextScheduledWidget)
    await flushPromises()
    expect(wrapper.text()).toContain('No upcoming backups.')
  })

  it('displays scheduled items when calendar has future events', async () => {
    mockGet.mockResolvedValue({
      data: [
        {
          date: '2026-06-01',
          events: [
            { type: 'backup', status: 'scheduled', repo_name: 'daily-repo', time: '03:00', schedule_id: 1 },
          ],
        },
      ],
    })
    const wrapper = renderWithPlugins(NextScheduledWidget)
    await flushPromises()
    expect(wrapper.text()).toContain('daily-repo')
  })
})
