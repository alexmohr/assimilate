// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'
import { mockErrorUtils, mockFormatBytes } from '../test-utils/sharedMocks'

vi.mock('../api/serverQuotas', () => ({
  listServerQuotas: vi.fn(),
  upsertServerQuota: vi.fn(),
  deleteServerQuota: vi.fn(),
}))

vi.mock('../utils/format', () => mockFormatBytes())

vi.mock('../utils/error', () => mockErrorUtils())

vi.mock('../components/BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

vi.mock('../components/ToggleSwitch.vue', () => ({
  default: {
    template: `<input type="checkbox" :checked="modelValue" @change="$emit('update:modelValue', $event.target.checked)" />`,
    props: ['modelValue'],
    emits: ['update:modelValue'],
  },
}))

const wsHandlers: Record<string, (payload: unknown) => void> = {}
vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    onMessage: (type: string, cb: (payload: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
}))

import { listServerQuotas, upsertServerQuota, deleteServerQuota } from '../api/serverQuotas'
import { renderWithPlugins } from '../test-utils'
import ServerQuotasView from './ServerQuotasView.vue'

const mockList = vi.mocked(listServerQuotas)
const mockUpsert = vi.mocked(upsertServerQuota)
const mockDelete = vi.mocked(deleteServerQuota)

const configuredQuota = {
  ssh_host: 'backup.example.com',
  repo_count: 2,
  total_deduplicated_size: 5_368_709_120,
  configured: true,
  warn_bytes: 8_589_934_592,
  critical_bytes: 10_737_418_240,
  warn_action: 'notify_only' as const,
  critical_action: 'block_backups' as const,
  enabled: true,
  updated_at: '2026-07-01T00:00:00Z',
}

const unconfiguredQuota = {
  ssh_host: 'other.example.com',
  repo_count: 1,
  total_deduplicated_size: 1_073_741_824,
  configured: false,
  warn_bytes: null,
  critical_bytes: null,
  warn_action: 'notify_only' as const,
  critical_action: 'notify_only' as const,
  enabled: false,
  updated_at: null,
}

describe('ServerQuotasView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    for (const key of Object.keys(wsHandlers)) {
      delete wsHandlers[key]
    }
  })

  it('shows loading state initially', async () => {
    mockList.mockReturnValue(new Promise(() => {}))
    const wrapper = renderWithPlugins(ServerQuotasView)
    await nextTick()
    expect(wrapper.find('.base-spinner').exists()).toBe(true)
  })

  it('lists hosts with usage and configured actions in cards', async () => {
    mockList.mockResolvedValue([configuredQuota, unconfiguredQuota])
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    expect(wrapper.find('table').exists()).toBe(false)
    expect(wrapper.findAll('.quota-card')).toHaveLength(2)
    expect(wrapper.text()).toContain('backup.example.com')
    expect(wrapper.text()).toContain('other.example.com')
    expect(wrapper.text()).toContain('Block backups')
    expect(wrapper.text()).toContain('Not set')
  })

  it('shows an empty state when no repos exist', async () => {
    mockList.mockResolvedValue([])
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain('No repositories are configured yet.')
  })

  it('shows an error message when loading fails', async () => {
    mockList.mockRejectedValue(new Error('network error'))
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain('API error')
  })

  it('saves a new quota configuration for an unconfigured host', async () => {
    mockList.mockResolvedValue([unconfiguredQuota])
    mockUpsert.mockResolvedValue({ ...unconfiguredQuota, configured: true, warn_bytes: 1 })
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    await wrapper.find('button.btn-ghost').trigger('click')
    await nextTick()

    await wrapper.find('#warn-gb').setValue(5)
    await wrapper.find('#warn-action').setValue('block_backups')
    await wrapper.find('#critical-gb').setValue(10)
    await wrapper.find('#critical-action').setValue('disable_schedule')
    await wrapper.find('.toggle-row input[type="checkbox"]').setValue(false)
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockUpsert).toHaveBeenCalledWith(
      'other.example.com',
      expect.objectContaining({
        warn_action: 'block_backups',
        critical_action: 'disable_schedule',
        enabled: false,
      }),
    )
  })

  it('shows an error and lets the user cancel the edit modal', async () => {
    mockList.mockResolvedValue([configuredQuota])
    mockUpsert.mockRejectedValue(new Error('save failed'))
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    await wrapper.find('button.btn-ghost').trigger('click')
    await nextTick()

    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(wrapper.text()).toContain('API error')

    await wrapper.find('button.close-btn').trigger('click')
    await nextTick()

    expect(wrapper.find('.dialog').exists()).toBe(false)
  })

  it('removes a configured quota', async () => {
    mockList.mockResolvedValue([configuredQuota])
    mockDelete.mockResolvedValue(undefined)
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    await wrapper.find('button.btn-danger-text').trigger('click')
    await flushPromises()

    expect(mockDelete).toHaveBeenCalledWith('backup.example.com')
  })

  it('reloads usage totals when a DataChanged event arrives (e.g. after a backup completes)', async () => {
    mockList.mockResolvedValue([configuredQuota])
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()
    expect(wrapper.text()).toContain(`${configuredQuota.total_deduplicated_size} B`)

    const grownQuota = { ...configuredQuota, total_deduplicated_size: 999_999_999_999 }
    mockList.mockResolvedValue([grownQuota])
    wsHandlers['DataChanged']?.(undefined)
    await flushPromises()

    expect(mockList).toHaveBeenCalledTimes(2)
    expect(wrapper.text()).toContain('999999999999 B')
  })
})
