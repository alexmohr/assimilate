// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  expectSavedEmitted,
  expectToggleOffThenOnResetsDependentOnSave,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { apiClient } from '../api/client'
import AgentPowerCard from './AgentPowerCard.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), get: vi.fn() },
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

describe('AgentPowerCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: AGENT } as never)
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] } as never)
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
    await startEditingSection(wrapper)

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
  // The wake details outlive the host's own toggle now that a schedule can
  // wake a host the toggle leaves alone: hiding them would put the MAC
  // address a job still wakes this host with out of reach.
  it('keeps the wake details on screen once the wake toggle is switched off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#power-wake-mac').exists()).toBe(true)

    const wakeToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[0]!
    await wakeToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#power-wake-mac').exists()).toBe(true)
  })

  it('hides the agent-process-dependent fields once start-agent is switched off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#power-service-name').exists()).toBe(true)

    const startAgentToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[2]!
    await startAgentToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#power-service-name').exists()).toBe(false)
  })

  // Shutting a host down always goes over SSH, independent of whether the
  // agent itself needs starting - a wake-only host (agent already runs as a
  // persistent service) still needs the SSH host field to configure that.
  it('keeps the SSH host field visible for a wake+shutdown host with start-agent off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)

    const startAgentToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[2]!
    await startAgentToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#power-ssh-host').exists()).toBe(true)
    expect(wrapper.find('#power-service-name').exists()).toBe(false)
  })

  it('hides the SSH host field when neither shutdown nor start-agent needs it', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)

    const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
    await toggles[1]!.vm.$emit('update:modelValue', false) // shutdownAfterBackup
    await toggles[2]!.vm.$emit('update:modelValue', false) // startAgentEnabled
    await flushPromises()

    expect(wrapper.find('#power-ssh-host').exists()).toBe(false)
  })

  // A value hidden by the toggle that gated it must not silently resubmit -
  // the server rejects `shutdown_after_backup: true` once `wake_enabled` is
  // false, and the field that could fix it is no longer on screen.
  // Same regression as before, hanging off the field that now gates a
  // shutdown: the server rejects `shutdown_after_backup: true` with no MAC
  // address to wake the host with (`agents_shutdown_requires_mac`).
  it('resets shutdown-after-backup once the MAC address is cleared', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#power-wake-mac').setValue('')
    await flushPromises()
    await clickSectionButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls.at(-1)?.[1]).toEqual(
      expect.objectContaining({
        wake: expect.objectContaining({ shutdown_after_backup: false }),
      }),
    )
  })

  it('resets stop-agent-after-backup once start-agent is switched off', async () => {
    const wrapper = mount()
    await expectToggleOffThenOnResetsDependentOnSave(
      wrapper,
      2,
      vi.mocked(apiClient.put),
      expect.objectContaining({ stop_agent_after_backup: false }),
    )
  })

  it('sends the whole power settings object on save', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#power-wake-mac').setValue('AA:BB:CC:DD:EE:FF')
    await clickSectionButton(wrapper, 'Save')

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

  it('sends edits to every field and toggle on save', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#power-wake-broadcast').setValue('10.0.0.255')
    await wrapper.find('#power-wake-timeout').setValue('300')
    await wrapper.find('#power-ssh-host').setValue('web-01.internal')
    await wrapper.find('#power-ssh-port').setValue('2222')
    await wrapper.find('#power-service-name').setValue('borg-agent')

    const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })
    await toggles[1]!.vm.$emit('update:modelValue', false) // shutdownAfterBackup
    await toggles[3]!.vm.$emit('update:modelValue', false) // stopAgentAfterBackup
    await flushPromises()

    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01/power',
      {
        wake: {
          wake_enabled: true,
          wake_mac_address: '3C:97:0E:2B:9A:44',
          wake_broadcast_address: '10.0.0.255',
          wake_timeout_seconds: 300,
          shutdown_after_backup: false,
        },
        start_agent_enabled: true,
        stop_agent_after_backup: false,
        ssh_host: 'web-01.internal',
        ssh_port: 2222,
        agent_service_name: 'borg-agent',
      },
      { params: {} },
    )
  })

  // Silently substituting a default here would let an admin clear the field
  // to retype a different unit name, save early, and never notice the wrong
  // value was persisted instead - the server rejects an empty name outright
  // when start_agent_enabled, same as it does for the other required fields.
  it('sends an emptied service name as-is rather than substituting the default', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#power-service-name').setValue('   ')

    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01/power',
      expect.objectContaining({ agent_service_name: '' }),
      { params: {} },
    )
  })

  it('emits the saved agent so the view can merge it back', async () => {
    const wrapper = mount()
    await expectSavedEmitted(wrapper, [AGENT])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    const wrapper = mount()
    await expectSaveErrorKeepsEditing(wrapper, 'agent offline', '#power-wake-mac')
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Cancel')

    expect(wrapper.find('#power-wake-mac').exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  // Switching this host's own wake off no longer means the host is never
  // woken, so the pane has to name the jobs that still wake it.
  it('names the schedules that wake this host whatever the setting says', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [
        {
          id: 7,
          name: 'Nightly workstations',
          wake_override: 'enabled',
          target_hostnames: ['web-01'],
        },
        {
          id: 8,
          name: 'Weekly media',
          wake_override: 'host_default',
          target_hostnames: ['web-01'],
        },
        { id: 9, name: 'Other host', wake_override: 'enabled', target_hostnames: ['db-01'] },
      ],
    } as never)
    const wrapper = mount()
    await flushPromises()

    const links = wrapper.findAll('.override-link')
    expect(links.map((l) => l.text())).toEqual(['Nightly workstations'])
    expect(wrapper.text()).toContain('1 schedule wakes this host')
  })

  it('leaves the override note out when no schedule overrides this host', async () => {
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.findAll('.override-link')).toHaveLength(0)
  })

  // The note is a courtesy, not the point of the pane: a schedules request
  // that fails must leave it out rather than take the power settings with it.
  it('still renders the settings when the schedules request fails', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.findAll('.override-link')).toHaveLength(0)
    expect(wrapper.text()).toContain('3C:97:0E:2B:9A:44')
  })
})
