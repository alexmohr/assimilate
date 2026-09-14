// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  expectSavedEmitted,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { apiClient } from '../api/client'
import RepoPowerCard from './RepoPowerCard.vue'
import type { RepoWithStats } from '../types/repo'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), get: vi.fn() },
}))

const REPO = {
  id: 42,
  power: {
    wake_enabled: true,
    wake_mac_address: '9C:B6:D0:1A:44:7F',
    wake_broadcast_address: '192.168.1.255',
    wake_timeout_seconds: 240,
    shutdown_after_backup: true,
  },
} as unknown as RepoWithStats

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoPowerCard, {
    props: { repo: REPO, isAdmin: true, ...props },
  })
}

describe('RepoPowerCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: REPO } as never)
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] } as never)
  })

  it('summarizes the current settings in view mode', () => {
    const text = mount().text()
    expect(text).toContain('9C:B6:D0:1A:44:7F')
    expect(text).toContain('192.168.1.255')
    expect(text).toContain('240 seconds')
  })

  it('omits the dependent rows when wake is off', () => {
    const wrapper = mount({
      repo: {
        ...REPO,
        power: {
          wake_enabled: false,
          wake_mac_address: null,
          wake_broadcast_address: null,
          wake_timeout_seconds: 180,
          shutdown_after_backup: false,
        },
      },
    })
    expect(wrapper.text()).not.toContain('MAC address')
  })

  // See AgentPowerCard.test.ts: an address missing while a shutdown is on has
  // been redacted for this viewer, not left unconfigured -
  // `repos_shutdown_requires_mac` guarantees one exists.
  it('says a redacted address is hidden rather than absent', () => {
    const text = mount({
      repo: {
        ...REPO,
        power: {
          ...REPO.power,
          wake_enabled: false,
          wake_mac_address: null,
          wake_broadcast_address: null,
          shutdown_after_backup: true,
        },
      },
    }).text()

    expect(text).toContain('Hidden')
    expect(text).not.toContain('Not set')
  })

  it('hides the Edit button for a non-admin', () => {
    const buttons = mount({ isAdmin: false }).findAll('button')
    expect(buttons.map((b) => b.text().trim())).not.toContain('Edit')
  })

  it('discloses the wake explanation in view mode', async () => {
    const wrapper = mount()
    await wrapper.find('[aria-label="Help: wake host before backup"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('its Settings, in Power.')
  })

  it('discloses each wake-related explanation on demand while editing', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)

    await wrapper.find('[aria-label="Help: wake host before backup"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('Test connection')

    await wrapper.find('[aria-label="Help: where the wake packet is sent"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('under its own Power settings')

    await wrapper.find('[aria-label="Help: the reconnect deadline"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toBe(
      'How long to wait for SSH before the backup is marked failed.',
    )

    await wrapper.find('[aria-label="Help: shut down host after backup"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain(
      'other schedules may still be writing to it',
    )
  })

  it('seeds every field from the current value when editing starts', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)

    expect(wrapper.find<HTMLInputElement>('#repo-power-wake-mac').element.value).toBe(
      '9C:B6:D0:1A:44:7F',
    )
    expect(wrapper.find<HTMLInputElement>('#repo-power-wake-broadcast').element.value).toBe(
      '192.168.1.255',
    )
    expect(wrapper.find<HTMLInputElement>('#repo-power-wake-timeout').element.value).toBe('240')
  })

  // See AgentPowerCard.test.ts: the wake details outlive the host's own
  // toggle, since a schedule can wake a host the toggle leaves alone.
  it('keeps the wake details on screen once the wake toggle is switched off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(true)

    const wakeToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[0]!
    await wakeToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(true)
  })

  // A value whose precondition is gone must not silently resubmit - the
  // server rejects `shutdown_after_backup: true` with no MAC address to wake
  // the host with (`repos_shutdown_requires_mac`).
  it('resets shutdown-after-backup once the MAC address is cleared', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#repo-power-wake-mac').setValue('')
    await flushPromises()
    await clickSectionButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls.at(-1)?.[1]).toEqual(
      expect.objectContaining({ shutdown_after_backup: false }),
    )
  })

  it('sends the power settings object on save', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#repo-power-wake-mac').setValue('AA:BB:CC:DD:EE:FF')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledTimes(1)
    expect(apiClient.put).toHaveBeenCalledWith('/repos/42/power', {
      wake_enabled: true,
      wake_mac_address: 'AA:BB:CC:DD:EE:FF',
      wake_broadcast_address: '192.168.1.255',
      wake_timeout_seconds: 240,
      shutdown_after_backup: true,
    })
  })

  it('sends edits to every field and the shutdown toggle on save', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#repo-power-wake-broadcast').setValue('10.0.0.255')
    await wrapper.find('#repo-power-wake-timeout').setValue('300')

    const shutdownToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[1]!
    await shutdownToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/repos/42/power', {
      wake_enabled: true,
      wake_mac_address: '9C:B6:D0:1A:44:7F',
      wake_broadcast_address: '10.0.0.255',
      wake_timeout_seconds: 300,
      shutdown_after_backup: false,
    })
  })

  it('emits saved so the view can refetch', async () => {
    const wrapper = mount()
    await expectSavedEmitted(wrapper, [])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('host offline'))
    const wrapper = mount()
    await expectSaveErrorKeepsEditing(wrapper, 'host offline', '#repo-power-wake-mac')
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Cancel')

    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  // See AgentPowerCard.test.ts: switching this host's own wake off no longer
  // means the host is never woken.
  it('names the schedules that wake this host whatever the setting says', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [
        { id: 7, name: 'Nightly workstations', wake_override: 'enabled' },
        { id: 8, name: 'Weekly media', wake_override: 'host_default' },
      ],
    } as never)
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.findAll('.override-link').map((l) => l.text())).toEqual(['Nightly workstations'])
  })

  it('leaves the override note out when no schedule overrides this host', async () => {
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.findAll('.override-link')).toHaveLength(0)
  })

  // See AgentPowerCard.test.ts: a failed schedules request drops the note,
  // not the settings.
  it('still renders the settings when the schedules request fails', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.findAll('.override-link')).toHaveLength(0)
    expect(wrapper.text()).toContain('9C:B6:D0:1A:44:7F')
  })
})
