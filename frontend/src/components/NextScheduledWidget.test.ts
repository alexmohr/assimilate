// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import NextScheduledWidget from './NextScheduledWidget.vue'

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => `rel:${s}`,
  formatDuration: (n: number): string => `${n}s`,
}))

describe('NextScheduledWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(NextScheduledWidget, {
      props: {
        nextBackupAt: null,
        lastBackupAt: null,
        nextScheduleId: null,
        lastScheduleId: null,
      },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('displays em-dash when no backup times are set', () => {
    const wrapper = renderWithPlugins(NextScheduledWidget, {
      props: {
        nextBackupAt: null,
        lastBackupAt: null,
        nextScheduleId: null,
        lastScheduleId: null,
      },
    })
    expect(wrapper.text()).toContain('—')
  })

  it('displays relative times when backup times are provided', () => {
    const wrapper = renderWithPlugins(NextScheduledWidget, {
      props: {
        nextBackupAt: '2026-06-01T10:00:00Z',
        lastBackupAt: '2026-05-31T10:00:00Z',
        nextScheduleId: 42,
        lastScheduleId: 41,
      },
    })
    expect(wrapper.text()).toContain('rel:2026-06-01T10:00:00Z')
    expect(wrapper.text()).toContain('rel:2026-05-31T10:00:00Z')
  })
})
