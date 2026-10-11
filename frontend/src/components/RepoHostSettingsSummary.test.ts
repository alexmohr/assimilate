// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())

import { renderWithPlugins } from '../test-utils'
import { machine, machineWait } from '../test-utils/machineFixtures'
import { repoFixture } from '../test-utils/repoFixtures'
import { apiClient } from '../api/client'
import RepoHostSettingsSummary from './RepoHostSettingsSummary.vue'

const WAKING = repoFixture({
  power: {
    wake_enabled: true,
    wake_mac_address: '9C:B6:D0:1A:44:7F',
    wake_broadcast_address: null,
    wake_timeout_seconds: 240,
    shutdown_after_backup: true,
  },
})

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoHostSettingsSummary, {
    props: { repo: WAKING, isAdmin: true, ...props },
  })
}

describe('RepoHostSettingsSummary', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get)
      .mockReset()
      .mockResolvedValue({
        data: machine({
          availability: {
            intermittent: true,
            catch_up_recheck_minutes: 30,
            catch_up_give_up_minutes: 2880,
          },
          waiting: [
            machineWait(),
            // Another repository on the same host waits too, but not this one.
            machineWait({ schedule_id: 8, schedule_name: 'Media weekly', repo_id: 13 }),
          ],
        }),
      } as never)
  })

  it("shows the host's wake settings read-only, with no form of its own", async () => {
    const wrapper = mount()
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('backup.example.com')
    expect(text).toContain('9C:B6:D0:1A:44:7F')
    expect(text).toContain('240 seconds')
    expect(wrapper.findAll('input')).toHaveLength(0)
    expect(wrapper.findAll('button')).toHaveLength(0)
  })

  it("reads the availability from the host's machine, and what waits on this repository", async () => {
    const wrapper = mount()
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/machines/20')
    const text = wrapper.text()
    expect(text).toContain('Host is not always online')
    expect(text).toContain('30 minutes')
    expect(text).toContain('2 days')
    expect(wrapper.find('a[href="/schedules/7"]').text()).toBe('Nightly servers')
    expect(wrapper.find('a[href="/schedules/8"]').exists()).toBe(false)
  })

  it('links an admin to the host to change the settings', async () => {
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('a[href="/repo-hosts/5?section=power"]').exists()).toBe(true)
  })

  it('offers no link to a viewer who could not open the host', async () => {
    const wrapper = mount({ isAdmin: false })
    await flushPromises()
    expect(wrapper.find('a[href^="/repo-hosts/"]').exists()).toBe(false)
  })

  it('says a redacted MAC address is hidden', async () => {
    const wrapper = mount({
      repo: repoFixture({ power: { ...WAKING.power, wake_mac_address: null } }),
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Hidden')
  })

  it('reports a failed load in place of the availability', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('.state-error').text()).toContain('boom')
  })

  it('reloads the availability when it is shown for another repository', async () => {
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await wrapper.setProps({
      repo: { ...WAKING, id: 13, repo_host: { ...WAKING.repo_host, machine_id: 21 } },
    })
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/machines/21')
    expect(wrapper.text()).toContain('Media weekly')
  })
})
