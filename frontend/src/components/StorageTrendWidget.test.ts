// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import StorageTrendWidget from './StorageTrendWidget.vue'

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => s,
  formatDuration: (n: number): string => `${n}s`,
}))

interface StorageEntry {
  name: string
  deduplicated_size: number
  percentage: number
}

describe('StorageTrendWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { storageBreakdown: [], totalStorageBytes: 0 },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows empty state when no storage data', () => {
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { storageBreakdown: [], totalStorageBytes: 0 },
    })
    expect(wrapper.text()).toContain('No storage data available.')
  })

  it('displays storage entries with name and size', () => {
    const breakdown: StorageEntry[] = [
      { name: 'repo-alpha', deduplicated_size: 1_073_741_824, percentage: 60 },
      { name: 'repo-beta', deduplicated_size: 536_870_912, percentage: 40 },
    ]
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { storageBreakdown: breakdown, totalStorageBytes: 1_610_612_736 },
    })
    expect(wrapper.text()).toContain('repo-alpha')
    expect(wrapper.text()).toContain('repo-beta')
    expect(wrapper.text()).toContain('60%')
    expect(wrapper.text()).toContain('40%')
  })

  it('displays total storage via formatBytes', () => {
    const breakdown: StorageEntry[] = [
      { name: 'only-repo', deduplicated_size: 1024, percentage: 100 },
    ]
    const wrapper = renderWithPlugins(StorageTrendWidget, {
      props: { storageBreakdown: breakdown, totalStorageBytes: 1024 },
    })
    expect(wrapper.text()).toContain('1024B')
  })
})
