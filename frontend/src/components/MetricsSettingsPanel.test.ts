// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import MetricsSettingsPanel from './MetricsSettingsPanel.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback: string) => fallback,
}))

import { apiClient } from '../api/client'

const mockGet = vi.mocked(apiClient.get)
const mockPut = vi.mocked(apiClient.put)

beforeEach(() => {
  vi.clearAllMocks()
})

describe('MetricsSettingsPanel', () => {
  it('shows the stored settings, one network per line', async () => {
    mockGet.mockResolvedValue({
      data: { enabled: true, allowed_networks: ['10.0.0.0/8', '192.0.2.1/32'] },
    })

    const wrapper = renderWithPlugins(MetricsSettingsPanel)
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/system/metrics')
    expect(wrapper.find<HTMLInputElement>('#metrics-enabled').element.checked).toBe(true)
    expect(wrapper.find<HTMLTextAreaElement>('#metrics-networks').element.value).toBe(
      '10.0.0.0/8\n192.0.2.1/32',
    )
    expect(wrapper.text()).toContain('/metrics')
  })

  it('enables the endpoint and saves the networks as a list', async () => {
    mockGet.mockResolvedValue({ data: { enabled: false, allowed_networks: [] } })
    mockPut.mockResolvedValue({
      data: { enabled: true, allowed_networks: ['10.0.0.0/8', '127.0.0.1/32'] },
    })

    const wrapper = renderWithPlugins(MetricsSettingsPanel)
    await flushPromises()
    await wrapper.find('#metrics-enabled').setValue(true)
    await wrapper.find('#metrics-networks').setValue('10.0.0.0/8\n 127.0.0.1 \n')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith('/system/metrics', {
      enabled: true,
      allowed_networks: ['10.0.0.0/8', '127.0.0.1'],
    })
    expect(wrapper.find<HTMLTextAreaElement>('#metrics-networks').element.value).toBe(
      '10.0.0.0/8\n127.0.0.1/32',
    )
    expect(wrapper.find('.save-success').exists()).toBe(true)
  })

  it('explains a failed save and keeps what was typed', async () => {
    mockGet.mockResolvedValue({ data: { enabled: true, allowed_networks: [] } })
    mockPut.mockRejectedValue(new Error('invalid network'))

    const wrapper = renderWithPlugins(MetricsSettingsPanel)
    await flushPromises()
    await wrapper.find('#metrics-networks').setValue('everyone')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(wrapper.find('.state-error').text()).toContain('Failed to save metrics settings')
    expect(wrapper.find<HTMLTextAreaElement>('#metrics-networks').element.value).toBe('everyone')
    expect(wrapper.find('.save-success').exists()).toBe(false)
  })

  it('explains a failed load', async () => {
    mockGet.mockRejectedValue(new Error('forbidden'))

    const wrapper = renderWithPlugins(MetricsSettingsPanel)
    await flushPromises()

    expect(wrapper.find('.state-error').text()).toContain('Failed to load metrics settings')
  })
})
