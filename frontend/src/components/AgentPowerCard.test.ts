// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentPowerCard from './AgentPowerCard.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn() },
}))

const AGENT = {
  hostname: 'web-01',
  domain: null,
  power: {
    wake: {
      wake_enabled: true,
      wake_mac_address: '3C:97:0E:2B:9A:44',
      wake_broadcast_address: '192.168.1.255',
      wake_timeout_seconds: 180,
      shutdown_after_backup: true,
    },
    start_agent_enabled: true,
    stop_agent_after_backup: true,
    ssh_host: 'web-01.lan',
    ssh_port: 22,
    agent_service_name: 'assimilate-agent',
  },
} as unknown as AgentRow

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentPowerCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
}

async function startEditing(wrapper: ReturnType<typeof mount>) {
  await wrapper.find('button').trigger('click')
}

async function clickButton(wrapper: ReturnType<typeof mount>, label: string) {
  const button = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!button) throw new Error(`no "${label}" button on the card`)
  await button.trigger('click')
  await flushPromises()
}

describe('AgentPowerCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: AGENT } as never)
  })

  it('summarizes the current settings in view mode', () => {
    const text = mount().text()
    expect(text).toContain('3C:97:0E:2B:9A:44')
    expect(text).toContain('192.168.1.255')
    expect(text).toContain('180 seconds')
    expect(text).toContain('web-01.lan')
    expect(text).toContain('assimilate-agent')
  })

  it('omits the dependent rows when wake and start-agent are both off', () => {
    const wrapper = mount({
      agent: {
        ...AGENT,
        power: {
          wake: {
            wake_enabled: false,
            wake_mac_address: null,
            wake_broadcast_address: null,
            wake_timeout_seconds: 180,
            shutdown_after_backup: false,
          },
          start_agent_enabled: false,
          stop_agent_after_backup: false,
          ssh_host: null,
          ssh_port: 22,
          agent_service_name: 'assimilate-agent',
        },
      },
    })
    expect(wrapper.text()).not.toContain('MAC address')
    expect(wrapper.text()).not.toContain('Service name')
  })

  it('hides the Edit button for an imported host', () => {
    expect(mount({ canEdit: false }).findAll('button')).toHaveLength(0)
  })

  it('seeds every field from the current value when editing starts', async () => {
    const wrapper = mount()
    await startEditing(wrapper)

    expect(wrapper.find<HTMLInputElement>('#power-wake-mac').element.value).toBe(
      '3C:97:0E:2B:9A:44',
    )
    expect(wrapper.find<HTMLInputElement>('#power-wake-broadcast').element.value).toBe(
      '192.168.1.255',
    )
    expect(wrapper.find<HTMLInputElement>('#power-wake-timeout').element.value).toBe('180')
    expect(wrapper.find<HTMLInputElement>('#power-ssh-host').element.value).toBe('web-01.lan')
    expect(wrapper.find<HTMLInputElement>('#power-service-name').element.value).toBe(
      'assimilate-agent',
    )
  })

  // Both dependent groups (wake details, agent-process details) collapse
  // in the edit form the moment their toggle is switched off - the fields a
  // Save would otherwise send stale values for should not even be visible.
  it('hides the wake-dependent fields once the wake toggle is switched off', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    expect(wrapper.find('#power-wake-mac').exists()).toBe(true)

    const wakeToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[0]!
    await wakeToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#power-wake-mac').exists()).toBe(false)
  })

  it('hides the agent-process-dependent fields once start-agent is switched off', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    expect(wrapper.find('#power-service-name').exists()).toBe(true)

    const startAgentToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[2]!
    await startAgentToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#power-service-name').exists()).toBe(false)
  })

  it('sends the whole power settings object on save', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await wrapper.find('#power-wake-mac').setValue('AA:BB:CC:DD:EE:FF')
    await clickButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledTimes(1)
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01/power',
      {
        wake: {
          wake_enabled: true,
          wake_mac_address: 'AA:BB:CC:DD:EE:FF',
          wake_broadcast_address: '192.168.1.255',
          wake_timeout_seconds: 180,
          shutdown_after_backup: true,
        },
        start_agent_enabled: true,
        stop_agent_after_backup: true,
        ssh_host: 'web-01.lan',
        ssh_port: 22,
        agent_service_name: 'assimilate-agent',
      },
      { params: {} },
    )
  })

  it('emits the saved agent so the view can merge it back', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await clickButton(wrapper, 'Save')

    expect(wrapper.emitted('saved')).toEqual([[AGENT]])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    const wrapper = mount()
    await startEditing(wrapper)
    await clickButton(wrapper, 'Save')

    expect(wrapper.find('.form-error').text()).toContain('agent offline')
    expect(wrapper.find('#power-wake-mac').exists()).toBe(true)
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await clickButton(wrapper, 'Cancel')

    expect(wrapper.find('#power-wake-mac').exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })
})
