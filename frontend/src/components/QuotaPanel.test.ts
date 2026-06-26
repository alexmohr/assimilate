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
const mockPut = vi.mocked(apiClient.put)

describe('QuotaPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows loading state initially', async () => {
    mockGet.mockReturnValue(new Promise(() => {}))
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await nextTick()
    expect(wrapper.text()).toContain('Loading quota')
  })

  it('renders quota bar and usage in normal (ok) state', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 10_737_418_240,
        critical_bytes: 21_474_836_480,
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 1_073_741_824 },
    })
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
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 5_368_709_120 },
    })
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
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 6_442_450_944 },
    })
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
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('disabled')
  })

  it('shows Edit button for admin users', async () => {
    mockGet.mockResolvedValue({
      data: { warn_bytes: 0, critical_bytes: 0, enabled: true },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Edit')
  })

  it('does not show Edit button for non-admin users', async () => {
    mockGet.mockResolvedValue({
      data: { warn_bytes: 0, critical_bytes: 0, enabled: true },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).not.toContain('Edit')
  })

  it('shows error message when API fails', async () => {
    mockGet.mockRejectedValue(new Error('network error'))
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 0 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('API error')
  })

  it('displays warn and critical action labels', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 10_737_418_240,
        critical_bytes: 21_474_836_480,
        warn_action: 'block_backups',
        critical_action: 'disable_schedule',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 1_073_741_824 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Block all backups + notify')
    expect(wrapper.text()).toContain('Disable schedule + notify')
  })

  it('displays notify_only action label', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 10_737_418_240,
        critical_bytes: 21_474_836_480,
        warn_action: 'notify_only',
        critical_action: 'notify_only',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: false, currentUsageBytes: 1_073_741_824 },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Notify only')
  })

  it('edit form initializes action dropdowns from quota data', async () => {
    mockGet.mockResolvedValue({
      data: {
        warn_bytes: 10_737_418_240,
        critical_bytes: 21_474_836_480,
        warn_action: 'block_backups',
        critical_action: 'disable_schedule',
        enabled: true,
      },
    })
    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()

    const editBtn = wrapper.findAll('button').find((b) => b.text() === 'Edit')
    expect(editBtn).toBeTruthy()
    await editBtn!.trigger('click')
    await nextTick()

    const selects = wrapper.findAll('select')
    expect(selects).toHaveLength(2)
    expect((selects[0].element as HTMLSelectElement).value).toBe('block_backups')
    expect((selects[1].element as HTMLSelectElement).value).toBe('disable_schedule')
  })

  it('save sends warn_action and critical_action in PUT request', async () => {
    mockGet
      .mockResolvedValueOnce({
        data: {
          warn_bytes: 10_737_418_240,
          critical_bytes: 21_474_836_480,
          warn_action: 'notify_only',
          critical_action: 'notify_only',
          enabled: true,
        },
      })
      .mockResolvedValueOnce({
        data: {
          warn_bytes: 10_737_418_240,
          critical_bytes: 21_474_836_480,
          warn_action: 'block_backups',
          critical_action: 'disable_schedule',
          enabled: true,
        },
      })
    mockPut.mockResolvedValue({ data: {} })

    const wrapper = renderWithPlugins(QuotaPanel, {
      props: { repoId: 1, isAdmin: true, currentUsageBytes: 0 },
    })
    await flushPromises()

    const editBtn = wrapper.findAll('button').find((b) => b.text() === 'Edit')
    await editBtn!.trigger('click')
    await nextTick()

    const selects = wrapper.findAll('select')
    await selects[0].setValue('block_backups')
    await selects[1].setValue('disable_schedule')

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save')
    await saveBtn!.trigger('click')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      '/repos/1/quota',
      expect.objectContaining({
        warn_action: 'block_backups',
        critical_action: 'disable_schedule',
      }),
    )
  })
})
