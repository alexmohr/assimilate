// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import BackupStatsWidget from './BackupStatsWidget.vue'

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => s,
  formatDuration: (n: number): string => `${n}s`,
}))

describe('BackupStatsWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { successRate: 0, success30d: 0, failed30d: 0, total30d: 0 },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('displays the success rate percentage', () => {
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { successRate: 85, success30d: 17, failed30d: 3, total30d: 20 },
    })
    expect(wrapper.text()).toContain('85%')
  })

  it('displays passed and failed counts', () => {
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { successRate: 90, success30d: 27, failed30d: 3, total30d: 30 },
    })
    expect(wrapper.text()).toContain('27')
    expect(wrapper.text()).toContain('3')
  })

  it('shows 0% when no backups have run', () => {
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { successRate: 0, success30d: 0, failed30d: 0, total30d: 0 },
    })
    expect(wrapper.text()).toContain('0%')
    expect(wrapper.text()).toContain('0/0 OK')
  })
})
