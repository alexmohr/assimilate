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
import { machine } from '../test-utils/machineFixtures'
import { apiClient } from '../api/client'
import MachinePowerSection from './MachinePowerSection.vue'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), get: vi.fn() },
}))

const MACHINE = machine({
  power: {
    wake_enabled: true,
    wake_mac_address: '9C:B6:D0:1A:44:7F',
    wake_broadcast_address: '192.168.1.255',
    wake_timeout_seconds: 240,
    shutdown_after_backup: true,
  },
})

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(MachinePowerSection, {
    props: { machine: MACHINE, canEdit: true, ...props },
  })
}

describe('MachinePowerSection', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: MACHINE } as never)
  })

  it('summarizes the current settings in view mode', () => {
    const text = mount().text()
    expect(text).toContain('9C:B6:D0:1A:44:7F')
    expect(text).toContain('192.168.1.255')
    expect(text).toContain('240 seconds')
    expect(text).toContain('Shut down after backup')
  })

  it('omits the wake details when nothing wakes the machine', () => {
    const wrapper = mount({
      machine: machine({
        power: {
          wake_enabled: false,
          wake_mac_address: null,
          wake_broadcast_address: null,
          wake_timeout_seconds: 180,
          shutdown_after_backup: false,
        },
      }),
    })
    expect(wrapper.text()).not.toContain('MAC address')
  })

  // `machines_shutdown_requires_mac` guarantees an address exists while a
  // shutdown is on, so a missing one has been redacted for this viewer.
  it('says a redacted address is hidden rather than absent', () => {
    const text = mount({
      machine: machine({
        power: {
          ...MACHINE.power,
          wake_enabled: false,
          wake_mac_address: null,
          wake_broadcast_address: null,
          shutdown_after_backup: true,
        },
      }),
    }).text()

    expect(text).toContain('Hidden')
    expect(text).not.toContain('Not set')
  })

  it('hides the Edit button from a viewer who may not change it', () => {
    const buttons = mount({ canEdit: false }).findAll('button')
    expect(buttons.map((b) => b.text().trim())).not.toContain('Edit')
  })

  // A machine that is only a dependency is never shut down: the row is not
  // offered, rather than offered and refused by the server.
  it('leaves shutdown out for a machine that is only a dependency', async () => {
    const wrapper = mount({
      machine: machine({
        can_shut_down: false,
        roles: { agent: null, repo_host: null, dependencies: [{ id: 3, name: 'nas-media' }] },
        power: { ...MACHINE.power, shutdown_after_backup: false },
      }),
    })
    expect(wrapper.text()).not.toContain('Shut down after backup')
    expect(wrapper.text()).toContain('never shuts down a machine that is only a dependency')

    await startEditingSection(wrapper)
    expect(wrapper.findAllComponents({ name: 'ToggleSwitch' })).toHaveLength(1)
    await clickSectionButton(wrapper, 'Save')
    expect(vi.mocked(apiClient.put).mock.calls.at(-1)?.[1]).toEqual(
      expect.objectContaining({ shutdown_after_backup: false }),
    )
  })

  it('seeds every field from the current value when editing starts', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)

    expect(wrapper.find<HTMLInputElement>('#machine-wake-mac').element.value).toBe(
      '9C:B6:D0:1A:44:7F',
    )
    expect(wrapper.find<HTMLInputElement>('#machine-wake-broadcast').element.value).toBe(
      '192.168.1.255',
    )
    expect(wrapper.find<HTMLInputElement>('#machine-wake-timeout').element.value).toBe('240')
  })

  // A schedule can wake a machine its own toggle leaves alone, with this MAC.
  it('keeps the wake details on screen once the wake toggle is switched off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper
      .findAllComponents({ name: 'ToggleSwitch' })[0]!
      .vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#machine-wake-mac').exists()).toBe(true)
  })

  // The server rejects a shutdown with no MAC address to wake the machine with.
  it('resets shutdown-after-backup once the MAC address is cleared', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#machine-wake-mac').setValue('')
    await flushPromises()
    await clickSectionButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls.at(-1)?.[1]).toEqual(
      expect.objectContaining({ wake_mac_address: null, shutdown_after_backup: false }),
    )
  })

  it('saves every field to the machine and hands the saved machine back', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#machine-wake-broadcast').setValue('10.0.0.255')
    await wrapper.find('#machine-wake-timeout').setValue('300')
    await wrapper
      .findAllComponents({ name: 'ToggleSwitch' })[1]!
      .vm.$emit('update:modelValue', false)
    await flushPromises()
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/machines/20/power', {
      wake_enabled: true,
      wake_mac_address: '9C:B6:D0:1A:44:7F',
      wake_broadcast_address: '10.0.0.255',
      wake_timeout_seconds: 300,
      shutdown_after_backup: false,
    })
    expect(wrapper.emitted('saved')).toEqual([[MACHINE]])
    expect(wrapper.find('#machine-wake-mac').exists()).toBe(false)
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('machine offline'))
    await expectSaveErrorKeepsEditing(mount(), 'machine offline', '#machine-wake-mac')
  })

  it('leaves without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Cancel')

    expect(wrapper.find('#machine-wake-mac').exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('closes a half-edited form when it is shown for another machine', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.setProps({ machine: machine({ id: 21 }) })
    await flushPromises()

    expect(wrapper.find('#machine-wake-mac').exists()).toBe(false)
  })

  // Switching the machine's own wake off does not mean it is never woken:
  // the schedules that decide otherwise are named, with what they do.
  it('names the schedules that do not leave it to the setting', () => {
    const wrapper = mount({
      machine: machine({
        schedule_overrides: [
          {
            schedule_id: 7,
            schedule_name: 'Nightly',
            wake_override: 'host_default',
            mode: 'wait',
          },
          {
            schedule_id: 8,
            schedule_name: 'Weekly',
            wake_override: 'enabled',
            mode: null,
          },
        ],
      }),
    })

    const links = wrapper.findAll('.override-link')
    expect(links.map((l) => l.text())).toEqual([
      'Nightly · waits for it without waking it',
      'Weekly · wakes every machine it needs',
    ])
    expect(links[0]!.attributes('href')).toBe('/schedules/7?tab=settings&section=power')
    expect(wrapper.find('.override-lead').text()).toContain('2 schedules do not leave')
  })

  it('leaves the exceptions out when every schedule follows the machine', () => {
    expect(mount().find('.override-note').exists()).toBe(false)
  })

  it('discloses what the wake setting is the default for', async () => {
    const wrapper = mount()
    await wrapper.find('[aria-label="Help: wake before backup"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('Power & dependencies')
  })
})
