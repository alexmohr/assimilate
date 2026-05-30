// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RecentActivityWidget from './RecentActivityWidget.vue'

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => `rel:${s}`,
  formatDuration: (n: number): string => `${n}s`,
}))

interface ActivityEntry {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
}

describe('RecentActivityWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(RecentActivityWidget, {
      props: { activity: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows empty state when no activity', () => {
    const wrapper = renderWithPlugins(RecentActivityWidget, {
      props: { activity: [] },
    })
    expect(wrapper.text()).toContain('No recent activity.')
  })

  it('displays hostname and target for each activity entry', () => {
    const activity: ActivityEntry[] = [
      {
        id: 1,
        hostname: 'web-server-01',
        target_name: 'daily',
        started_at: '2026-05-31T03:00:00Z',
        finished_at: '2026-05-31T03:05:00Z',
        status: 'success',
        duration_secs: 300,
      },
    ]
    const wrapper = renderWithPlugins(RecentActivityWidget, { props: { activity } })
    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('daily')
  })

  it('displays duration for each activity entry', () => {
    const activity: ActivityEntry[] = [
      {
        id: 2,
        hostname: 'db-server-01',
        target_name: 'db-backup',
        started_at: '2026-05-31T01:00:00Z',
        finished_at: '2026-05-31T01:10:00Z',
        status: 'success',
        duration_secs: 600,
      },
    ]
    const wrapper = renderWithPlugins(RecentActivityWidget, { props: { activity } })
    expect(wrapper.text()).toContain('600s')
  })

  it('renders multiple entries', () => {
    const activity: ActivityEntry[] = [
      {
        id: 1,
        hostname: 'host-a',
        target_name: 'repo-a',
        started_at: '2026-05-31T02:00:00Z',
        finished_at: '2026-05-31T02:01:00Z',
        status: 'success',
        duration_secs: 60,
      },
      {
        id: 2,
        hostname: 'host-b',
        target_name: 'repo-b',
        started_at: '2026-05-31T01:00:00Z',
        finished_at: '2026-05-31T01:01:00Z',
        status: 'failed',
        duration_secs: 45,
      },
    ]
    const wrapper = renderWithPlugins(RecentActivityWidget, { props: { activity } })
    expect(wrapper.text()).toContain('host-a')
    expect(wrapper.text()).toContain('host-b')
  })
})
