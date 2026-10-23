// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import AgentChannelKey from './AgentChannelKey.vue'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn() },
}))

const { mockCopy } = vi.hoisted(() => ({ mockCopy: vi.fn() }))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: false, copy: mockCopy }),
}))

import { apiClient } from '../api/client'

const mockGet = vi.mocked(apiClient.get)

const KEY = 'q1Wn8tOrbTCQ3nG1Q2s2jrSyJYk3bHHZq5c8nUo9t0g='
const FINGERPRINT = 'SHA256:3v6dS8a7oD8bTn0w1Y0mUA'

describe('AgentChannelKey', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows the key and its fingerprint, and copies the key', async () => {
    mockGet.mockResolvedValue({ data: { public_key: KEY, fingerprint: FINGERPRINT } })

    const wrapper = renderWithPlugins(AgentChannelKey)
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/system/agent-channel-key')
    expect(wrapper.find('[data-testid="agent-channel-key"]').text()).toBe(KEY)
    expect(wrapper.text()).toContain('BORG_SERVER_KEY')
    expect(wrapper.text()).toContain(FINGERPRINT)

    await wrapper.find('button').trigger('click')
    expect(mockCopy).toHaveBeenCalledWith(KEY)
  })

  it('says so when the key cannot be loaded', async () => {
    mockGet.mockRejectedValue(new Error('boom'))

    const wrapper = renderWithPlugins(AgentChannelKey)
    await flushPromises()

    expect(wrapper.find('.state-error').exists()).toBe(true)
    expect(wrapper.find('[data-testid="agent-channel-key"]').exists()).toBe(false)
  })
})
