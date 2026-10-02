// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())

import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { dependencyHost, dependencyPower, infoGridRows } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyPowerCard from './DependencyPowerCard.vue'
import type { DependencyHostResponse } from '../types/generated'

const OWN = dependencyHost({
  power: dependencyPower({
    wake_enabled: true,
    wake_mac_address: '9C:B6:D0:1A:44:7F',
    wake_broadcast_address: '192.168.1.255',
    wake_timeout_seconds: 120,
    effective_wake_enabled: true,
    effective_wake_mac_address: '9C:B6:D0:1A:44:7F',
    effective_wake_broadcast_address: '192.168.1.255',
    effective_wake_timeout_seconds: 120,
  }),
})

/** Shares nas-01's settings, while its own (switched off) are still stored. */
const SHARED = dependencyHost({
  power: dependencyPower({
    repo_host: { id: 9, ssh_host: 'nas-01' },
    wake_enabled: false,
    wake_mac_address: 'AA:BB:CC:DD:EE:FF',
    wake_broadcast_address: null,
    wake_timeout_seconds: 60,
    effective_wake_enabled: true,
    effective_wake_mac_address: '9C:B6:D0:1A:44:7F',
    effective_wake_broadcast_address: null,
    effective_wake_timeout_seconds: 180,
  }),
})

const REPO_HOSTS = [
  { id: 9, ssh_host: 'nas-01' },
  { id: 12, ssh_host: 'backup-box' },
]

async function render(
  host: DependencyHostResponse,
  isAdmin = true,
): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(DependencyPowerCard, { props: { host, isAdmin } })
  await flushPromises()
  return wrapper
}

async function pick(wrapper: ReturnType<typeof renderWithPlugins>, label: string): Promise<void> {
  await wrapper
    .findAll('button')
    .find((b) => b.text() === label)!
    .trigger('click')
}

describe('DependencyPowerCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: REPO_HOSTS })
  })

  it('shows its own wake settings', async () => {
    const wrapper = await render(OWN)
    expect(infoGridRows(wrapper)).toEqual({
      'Power settings': 'Own wake settings',
      'Wake host before backup': 'Enabled',
      'MAC address': '9C:B6:D0:1A:44:7F',
      'Broadcast address': '192.168.1.255',
      'Wait for host': '120 seconds',
    })
    expect(wrapper.text()).toContain('Assimilate never shuts a dependency down.')
    expect(wrapper.text()).not.toContain('Edit these on')
  })

  it("shows a repository host's settings when they are shared, and where to change them", async () => {
    const wrapper = await render(SHARED)
    const rows = infoGridRows(wrapper)
    expect(rows['Power settings']).toBe('Shared with repository host nas-01')
    expect(rows['MAC address']).toBe('9C:B6:D0:1A:44:7F')
    expect(rows['Broadcast address']).toBe('Default')
    expect(rows['Wait for host']).toBe('180 seconds')
    expect(wrapper.get('a[href="/repo-hosts/9?section=power"]').text()).toBe('nas-01')
    expect(wrapper.text()).toContain("Edit these on nas-01's Power section.")
    expect(wrapper.text()).toContain('only once no run that needs this dependency is still going')
  })

  it('says withheld addresses are hidden rather than unset', async () => {
    const wrapper = await render(
      dependencyHost({ power: dependencyPower({ effective_wake_enabled: true }) }),
      false,
    )
    const rows = infoGridRows(wrapper)
    expect(rows['MAC address']).toBe('Hidden')
    expect(rows['Broadcast address']).toBe('Hidden')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('leaves the wake details out while waking is off', async () => {
    const wrapper = await render(dependencyHost())
    expect(infoGridRows(wrapper)).toEqual({
      'Power settings': 'Own wake settings',
      'Wake host before backup': 'Disabled',
    })
  })

  it('offers no way to shut a dependency down', async () => {
    const wrapper = await render(OWN)
    await startEditingSection(wrapper)
    expect(wrapper.text().toLowerCase()).not.toContain('shut down host')
    expect(wrapper.findAllComponents({ name: 'ToggleSwitch' })).toHaveLength(1)
  })

  it('saves its own settings', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: OWN })
    const wrapper = await render(OWN)
    await startEditingSection(wrapper)
    await wrapper.get('#dependency-power-mac').setValue(' 11:22:33:44:55:66 ')
    await wrapper.get('#dependency-power-broadcast').setValue('')
    await wrapper.get('#dependency-power-wait').setValue(90)
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/power', {
      repo_host_id: null,
      wake_enabled: true,
      wake_mac_address: '11:22:33:44:55:66',
      wake_broadcast_address: null,
      wake_timeout_seconds: 90,
    })
    expect(wrapper.emitted('saved')).toEqual([[OWN]])
  })

  it('keeps its own stored values while sharing a repository host', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: SHARED })
    const wrapper = await render(SHARED)
    await startEditingSection(wrapper)
    await flushPromises()
    expect(wrapper.find('#dependency-power-mac').exists()).toBe(false)

    await wrapper.get('#dependency-power-repo-host').findAll('option')[2]!.setSelected()
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/power', {
      repo_host_id: 12,
      wake_enabled: false,
      wake_mac_address: 'AA:BB:CC:DD:EE:FF',
      wake_broadcast_address: null,
      wake_timeout_seconds: 60,
    })
  })

  it('switches from shared to own settings, restoring the stored ones', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: OWN })
    const wrapper = await render(SHARED)
    await startEditingSection(wrapper)
    await pick(wrapper, 'Own wake settings')
    expect(wrapper.find('#dependency-power-repo-host').exists()).toBe(false)

    await wrapper.findComponent({ name: 'ToggleSwitch' }).vm.$emit('update:modelValue', true)
    await flushPromises()
    expect((wrapper.get('#dependency-power-mac').element as HTMLInputElement).value).toBe(
      'AA:BB:CC:DD:EE:FF',
    )
    await clickSectionButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls[0]?.[1]).toMatchObject({
      repo_host_id: null,
      wake_enabled: true,
      wake_mac_address: 'AA:BB:CC:DD:EE:FF',
    })
  })

  it('asks for a repository host before sharing one', async () => {
    const wrapper = await render(OWN)
    await startEditingSection(wrapper)
    await pick(wrapper, 'Same as a repository host')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).not.toHaveBeenCalled()
    expect(wrapper.get('.form-error').text()).toContain('Pick the repository host')
  })

  it('keeps editing and shows why a save failed', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('a MAC address is required'))
    const wrapper = await render(OWN)
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')
    expect(wrapper.get('.form-error').text()).toContain('a MAC address is required')
    expect(wrapper.find('#dependency-power-mac').exists()).toBe(true)
  })
})
