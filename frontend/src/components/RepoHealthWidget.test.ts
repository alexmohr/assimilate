// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RepoHealthWidget from './RepoHealthWidget.vue'

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => `rel:${s}`,
  formatDuration: (n: number): string => `${n}s`,
}))

interface HealthEntry {
  repo_id: number
  hostname: string
  target_name: string
  last_status: string | null
  last_backup_at: string | null
  is_overdue: boolean
  cron_expression: string | null
  schedule_enabled: boolean | null
}

describe('RepoHealthWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(RepoHealthWidget, {
      props: { health: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows empty state when no health entries', () => {
    const wrapper = renderWithPlugins(RepoHealthWidget, {
      props: { health: [] },
    })
    expect(wrapper.text()).toContain('No repositories configured yet.')
  })

  it('displays hostname and target name for each entry', () => {
    const health: HealthEntry[] = [
      {
        repo_id: 1,
        hostname: 'web-server-01',
        target_name: 'daily',
        last_status: 'success',
        last_backup_at: '2026-05-31T08:00:00Z',
        is_overdue: false,
        cron_expression: '0 3 * * *',
        schedule_enabled: true,
      },
    ]
    const wrapper = renderWithPlugins(RepoHealthWidget, { props: { health } })
    expect(wrapper.text()).toContain('web-server-01')
    expect(wrapper.text()).toContain('daily')
  })

  it('shows OVERDUE badge for overdue entries', () => {
    const health: HealthEntry[] = [
      {
        repo_id: 2,
        hostname: 'db-server-01',
        target_name: 'db-backup',
        last_status: null,
        last_backup_at: null,
        is_overdue: true,
        cron_expression: '0 1 * * *',
        schedule_enabled: true,
      },
    ]
    const wrapper = renderWithPlugins(RepoHealthWidget, { props: { health } })
    expect(wrapper.text()).toContain('OVERDUE')
  })

  it('displays Never when last_backup_at is null', () => {
    const health: HealthEntry[] = [
      {
        repo_id: 3,
        hostname: 'media-01',
        target_name: 'media',
        last_status: null,
        last_backup_at: null,
        is_overdue: false,
        cron_expression: null,
        schedule_enabled: null,
      },
    ]
    const wrapper = renderWithPlugins(RepoHealthWidget, { props: { health } })
    expect(wrapper.text()).toContain('Never')
  })
})
