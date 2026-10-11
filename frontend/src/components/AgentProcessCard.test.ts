// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { apiClient } from '../api/client'
import AgentProcessCard from './AgentProcessCard.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), get: vi.fn() },
}))

function agent(
  power: Partial<AgentRow['power']> = {},
  shutdownAfterBackup = false,
  domain: string | null = null,
): AgentRow {
  return {
    hostname: 'web-01',
    domain,
    power: {
      wake: {
        wake_enabled: true,
        wake_mac_address: '3C:97:0E:2B:9A:44',
        wake_broadcast_address: null,
        wake_timeout_seconds: 180,
        shutdown_after_backup: shutdownAfterBackup,
      },
      start_agent_enabled: true,
      stop_agent_after_backup: true,
      ssh_host: 'web-01.lan',
      ssh_port: 2222,
      agent_service_name: 'assimilate-agent',
      ...power,
    },
  } as unknown as AgentRow
}

const AGENT = agent()

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentProcessCard, { props: { agent: AGENT, canEdit: true, ...props } })
}

function toggles(wrapper: ReturnType<typeof mount>) {
  return wrapper.findAllComponents({ name: 'ToggleSwitch' })
}

describe('AgentProcessCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: AGENT } as never)
  })

  it('summarizes the current settings in view mode', () => {
    const text = mount().text()
    expect(text).toContain('web-01.lan:2222')
    expect(text).toContain('assimilate-agent')
    expect(text).toContain('Stop agent after backup')
  })

  it('omits the dependent rows when the agent is not started and the machine not shut down', () => {
    const text = mount({ agent: agent({ start_agent_enabled: false }) }).text()
    expect(text).not.toContain('SSH host')
    expect(text).not.toContain('Service name')
  })

  // The machine is shut down over the same SSH destination.
  it('shows the SSH host of a machine shut down after backup, with the agent not started', () => {
    const text = mount({ agent: agent({ start_agent_enabled: false }, true) }).text()
    expect(text).toContain('web-01.lan:2222')
    expect(text).not.toContain('Service name')
  })

  it('hides the Edit button for an imported host', () => {
    const buttons = mount({ canEdit: false }).findAll('button')
    expect(buttons.map((b) => b.text().trim())).not.toContain('Edit')
  })

  it('seeds every field from the current value when editing starts', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    expect(wrapper.find<HTMLInputElement>('#power-ssh-host').element.value).toBe('web-01.lan')
    expect(wrapper.find<HTMLInputElement>('#power-ssh-port').element.value).toBe('2222')
    expect(wrapper.find<HTMLInputElement>('#power-service-name').element.value).toBe(
      'assimilate-agent',
    )
  })

  it('hides the SSH host once neither starting the agent nor a shutdown needs it', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await toggles(wrapper)[0]!.vm.$emit('update:modelValue', false)
    await flushPromises()
    expect(wrapper.find('#power-ssh-host').exists()).toBe(false)
    expect(wrapper.find('#power-service-name').exists()).toBe(false)
  })

  it('keeps the SSH host for a shutdown once starting the agent is switched off', async () => {
    const wrapper = mount({ agent: agent({}, true) })
    await startEditingSection(wrapper)
    await toggles(wrapper)[0]!.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#power-ssh-host').exists()).toBe(true)
    expect(wrapper.find('#power-service-name').exists()).toBe(false)
    await wrapper.find('[aria-label="Help: needed for shutdown only"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('shut this machine down')
  })

  // The server refuses stopping an agent it was never asked to start.
  it('resets stop-agent-after-backup once start-agent is switched off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await toggles(wrapper)[0]!.vm.$emit('update:modelValue', false)
    await flushPromises()
    await clickSectionButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls.at(-1)?.[1]).toEqual(
      expect.objectContaining({ start_agent_enabled: false, stop_agent_after_backup: false }),
    )
  })

  it('saves the process settings, with the domain when there is one', async () => {
    const wrapper = mount({ agent: agent({}, false, 'lab') })
    await startEditingSection(wrapper)
    await wrapper.find('#power-ssh-host').setValue('  web-01.lab  ')
    await wrapper.find('#power-service-name').setValue('borg-agent')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01/process',
      {
        start_agent_enabled: true,
        stop_agent_after_backup: true,
        ssh_host: 'web-01.lab',
        ssh_port: 2222,
        agent_service_name: 'borg-agent',
      },
      { params: { domain: 'lab' } },
    )
    expect(wrapper.emitted('saved')).toEqual([[AGENT]])
  })

  it('sends an emptied SSH host as none', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#power-ssh-host').setValue('   ')
    await clickSectionButton(wrapper, 'Save')
    expect(vi.mocked(apiClient.put).mock.calls.at(-1)?.[1]).toEqual(
      expect.objectContaining({ ssh_host: null }),
    )
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('deploy the SSH key first'))
    await expectSaveErrorKeepsEditing(mount(), 'deploy the SSH key first', '#power-ssh-host')
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Cancel')
    expect(wrapper.find('#power-ssh-host').exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('discloses what starting and stopping the agent do', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('[aria-label="Help: start agent before backup"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('runs on demand')
    await wrapper.find('[aria-label="Help: stop agent after backup"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toBe('Only if this run started it.')
  })
})
