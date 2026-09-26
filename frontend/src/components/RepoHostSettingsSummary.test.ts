// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())

import { renderWithPlugins } from '../test-utils'
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
        data: {
          intermittent: true,
          catch_up_recheck_minutes: 30,
          catch_up_give_up_minutes: 2880,
          waiting: [
            {
              schedule_id: 7,
              schedule_name: 'Nightly servers',
              repo_id: 12,
              repo_name: 'server-daily',
              pending_for: '2026-09-26T02:00:00Z',
              last_probe_at: null,
              next_probe_at: '2026-09-26T02:30:00Z',
              give_up_at: null,
            },
          ],
        },
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

  it("reads the host's availability through the repository, and what waits on it", async () => {
    const wrapper = mount()
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/12/availability')
    const text = wrapper.text()
    expect(text).toContain('Host is not always online')
    expect(text).toContain('30 minutes')
    expect(text).toContain('2 days')
    expect(wrapper.find('a[href="/schedules/7"]').text()).toBe('Nightly servers')
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
})
