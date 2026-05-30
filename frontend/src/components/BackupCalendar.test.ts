// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import BackupCalendar from './BackupCalendar.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn().mockResolvedValue({ data: [] }),
  },
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

describe('BackupCalendar', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows loading state initially', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Loading')
  })

  it('displays the panel title', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Backup Calendar')
  })

  it('renders repo options in select', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
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

  it('renders navigation buttons for month switching', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    const buttons = wrapper.findAll('button')
    expect(buttons.length).toBeGreaterThanOrEqual(2)
  })
})
