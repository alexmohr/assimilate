// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { apiClient } from '../api/client'
import RepoPowerCard from './RepoPowerCard.vue'
import type { RepoWithStats } from '../types/repo'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn() },
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

  it('hides the Edit button for a non-admin', () => {
    expect(mount({ isAdmin: false }).findAll('button')).toHaveLength(0)
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

  it('hides the wake-dependent fields once the wake toggle is switched off', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(true)

    const wakeToggle = wrapper.findAllComponents({ name: 'ToggleSwitch' })[0]!
    await wakeToggle.vm.$emit('update:modelValue', false)
    await flushPromises()

    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(false)
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

  it('emits saved so the view can refetch', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')

    expect(wrapper.emitted('saved')).toEqual([[]])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('host offline'))
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')

    expect(wrapper.find('.form-error').text()).toContain('host offline')
    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(true)
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Cancel')

    expect(wrapper.find('#repo-power-wake-mac').exists()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })
})
