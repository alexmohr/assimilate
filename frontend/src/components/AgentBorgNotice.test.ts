// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentBorgNotice from './AgentBorgNotice.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { post: vi.fn() },
}))

function agent(borg: AgentRow['borg'], isConnected = true): AgentRow {
  return { hostname: 'bot', domain: null, is_connected: isConnected, borg } as unknown as AgentRow
}

const MISSING = { state: 'missing', binary: 'borg' } as const

function mount(row: AgentRow, canCheck = true) {
  return renderWithPlugins(AgentBorgNotice, { props: { agent: row, canCheck } })
}

describe('AgentBorgNotice', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post).mockReset()
  })

  it('warns that borg is not installed and how to fix it', () => {
    const wrapper = mount(agent(MISSING))
    const notice = wrapper.get('[data-testid="borg-notice"]')
    expect(notice.text()).toContain('Borg is not installed on this host')
    expect(notice.text()).toContain('BORG_BINARY')
    expect(notice.text()).toContain('apt install borgbackup')
  })

  it('shows nothing for a working borg, an unreported one, or an offline host', () => {
    for (const row of [
      agent({ state: 'available', version: '1.4.1' }),
      agent(null),
      agent(null, false),
    ]) {
      expect(mount(row).find('[data-testid="borg-notice"]').exists()).toBe(false)
    }
  })

  it('asks the agent to look again and waits for its answer', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})
    const wrapper = mount(agent(MISSING))

    await wrapper.get('button').trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/bot/check-borg', {}, { params: {} })
    expect(wrapper.get('button').text()).toBe('Checking...')
    expect(wrapper.get('button').attributes('disabled')).toBeDefined()

    await wrapper.setProps({ agent: agent({ state: 'broken', binary: 'borg', reason: 'no' }) })
    expect(wrapper.get('button').text()).toBe('Check again')
  })

  it('keeps waiting through a refetch that brings the same status, then gives up', async () => {
    vi.useFakeTimers()
    try {
      vi.mocked(apiClient.post).mockResolvedValue({})
      const wrapper = mount(agent(MISSING))

      await wrapper.get('button').trigger('click')
      await flushPromises()
      await wrapper.setProps({ agent: agent({ ...MISSING }) })
      expect(wrapper.get('button').text()).toBe('Checking...')

      await vi.advanceTimersByTimeAsync(5000)
      expect(wrapper.get('button').text()).toBe('Check again')
    } finally {
      vi.useRealTimers()
    }
  })

  it('shows why the check could not be asked for', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent not connected'))
    const wrapper = mount(agent(MISSING))

    await wrapper.get('button').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('agent not connected')
    expect(wrapper.get('button').text()).toBe('Check again')
  })

  it('leaves the button to those who may use it, and off for an offline host', () => {
    expect(mount(agent(MISSING), false).find('button').exists()).toBe(false)
    expect(mount(agent(MISSING, false)).get('button').attributes('disabled')).toBeDefined()
  })
})
