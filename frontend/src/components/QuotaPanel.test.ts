// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'
import { renderWithPlugins } from '../test-utils'
import QuotaPanel from './QuotaPanel.vue'
import { apiClient } from '../api/client'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (bytes: number): string => `${bytes} B`,
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
}))

vi.mock('./ToggleSwitch.vue', () => ({
  default: { template: '<input type="checkbox" />', props: ['modelValue'] },
}))

const mockGet = vi.mocked(apiClient.get)

describe('QuotaPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows loading state initially', async () => {
    mockGet.mockReturnValue(new Promise(() => {}))
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await nextTick()
    expect(wrapper.text()).toContain('Loading quota')
  })

  it('renders quota bar and usage in normal (ok) state', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 10_737_418_240,
        critical_bytes: 21_474_836_480,
        enabled: true,
        current_usage_bytes: 1_073_741_824,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await flushPromises()
    expect(wrapper.find('.progress-bar-fill').exists()).toBe(true)
    expect(wrapper.find('.bar-ok').exists()).toBe(true)
    expect(wrapper.find('.badge-ok').exists()).toBe(true)
    expect(wrapper.text()).toContain('OK')
  })

  it('renders warning state when usage exceeds warn threshold', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 1_073_741_824,
        critical_bytes: 10_737_418_240,
        enabled: true,
        current_usage_bytes: 5_368_709_120,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await flushPromises()
    expect(wrapper.find('.bar-warn').exists()).toBe(true)
    expect(wrapper.find('.badge-warn').exists()).toBe(true)
    expect(wrapper.text()).toContain('Warning')
  })

  it('renders critical state when usage exceeds critical threshold', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 1_073_741_824,
        critical_bytes: 5_368_709_120,
        enabled: true,
        current_usage_bytes: 6_442_450_944,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await flushPromises()
    expect(wrapper.find('.bar-crit').exists()).toBe(true)
    expect(wrapper.find('.badge-crit').exists()).toBe(true)
    expect(wrapper.text()).toContain('Critical')
  })

  it('shows disabled message when quota is not enabled', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 0,
        critical_bytes: 0,
        enabled: false,
        current_usage_bytes: 0,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await flushPromises()
    expect(wrapper.text()).toContain('disabled')
  })

  it('shows Edit button for admin users', async () => {
    mockGet.mockResolvedValue({
      data: { warn_bytes: 0, critical_bytes: 0, enabled: true, current_usage_bytes: 0 },
    })
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: true } })
    await flushPromises()
    expect(wrapper.text()).toContain('Edit')
  })

  it('does not show Edit button for non-admin users', async () => {
    mockGet.mockResolvedValue({
      data: { warn_bytes: 0, critical_bytes: 0, enabled: true, current_usage_bytes: 0 },
    })
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await flushPromises()
    expect(wrapper.text()).not.toContain('Edit')
  })

  it('shows error message when API fails', async () => {
    mockGet.mockRejectedValue(new Error('network error'))
    const wrapper = renderWithPlugins(QuotaPanel, { props: { repoId: 1, isAdmin: false } })
    await flushPromises()
    expect(wrapper.text()).toContain('API error')
  })
})
