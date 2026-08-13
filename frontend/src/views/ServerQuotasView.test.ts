// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { nextTick, ref } from 'vue'
import { mockErrorUtils, mockFormatBytes } from '../test-utils/sharedMocks'

vi.mock('../api/serverQuotas', () => ({
  listServerQuotas: vi.fn(),
  upsertServerQuota: vi.fn(),
  deleteServerQuota: vi.fn(),
}))

const isMobileRef = ref(false)
vi.mock('../composables/useMobile', () => ({
  useMobile: () => ({ isMobile: isMobileRef }),
}))

vi.mock('../utils/format', () => mockFormatBytes())

vi.mock('../utils/error', () => mockErrorUtils())

vi.mock('../components/BaseSpinner.vue', () => ({
  default: { template: '<div class="base-spinner" />' },
}))

vi.mock('../components/ToggleSwitch.vue', () => ({
  default: { template: '<input type="checkbox" />', props: ['modelValue'] },
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
    isMobileRef.value = false
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

  it('lists hosts with usage and configured actions', async () => {
    mockList.mockResolvedValue([configuredQuota, unconfiguredQuota])
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

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

    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockUpsert).toHaveBeenCalledWith(
      'other.example.com',
      expect.objectContaining({ warn_action: 'notify_only', critical_action: 'notify_only' }),
    )
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

  it('renders a card list instead of a table on narrow viewports', async () => {
    isMobileRef.value = true
    mockList.mockResolvedValue([configuredQuota, unconfiguredQuota])
    const wrapper = renderWithPlugins(ServerQuotasView)
    await flushPromises()

    expect(wrapper.find('table').exists()).toBe(false)
    expect(wrapper.findAll('.quota-card')).toHaveLength(2)
    expect(wrapper.text()).toContain('backup.example.com')
    expect(wrapper.text()).toContain('Block backups')
    expect(wrapper.text()).toContain('Not set')
  })

  it('removes a configured quota from the mobile card list', async () => {
    isMobileRef.value = true
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
