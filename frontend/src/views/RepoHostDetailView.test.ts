// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { ref } from 'vue'
import { mockApiClientRw, mockToast } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../composables/useToast', () => mockToast())

const wsHandlers: Record<string, (payload: unknown) => void> = {}
vi.mock('../composables/useWebSocket', () => ({
  useWebSocket: () => ({
    status: ref('connected'),
    onMessage: (type: string, cb: (p: unknown) => void) => {
      wsHandlers[type] = cb
    },
  }),
}))

import { renderWithPlugins } from '../test-utils'
import { dialogButton } from '../test-utils/dom'
import { apiClient } from '../api/client'
import RepoHostDetailView from './RepoHostDetailView.vue'
import type { RepoHost } from '../api/repoHosts'

function host(overrides: Partial<RepoHost> = {}): RepoHost {
  return {
    id: 5,
    ssh_host: 'nas.lan',
    ssh_port: 22,
    ssh_host_key: 'ssh-ed25519 AAAAPINNED',
    intermittent: true,
    power: {
      wake_enabled: true,
      wake_mac_address: '9C:B6:D0:1A:44:7F',
      wake_broadcast_address: null,
      wake_timeout_seconds: 180,
      shutdown_after_backup: false,
    },
    repositories: [
      { id: 1, name: 'inhouse-global', ssh_user: 'root', repo_path: '/mnt/borg', enabled: true },
      { id: 2, name: 'laptops', ssh_user: 'borg', repo_path: '/mnt/laptops', enabled: true },
    ],
    ...overrides,
  }
}

let current: RepoHost

function setupApi(value: RepoHost = host()): void {
  current = value
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/repo-hosts/5') return Promise.resolve({ data: current }) as never
    if (url === '/repo-hosts/5/availability') {
      return Promise.resolve({
        data: {
          intermittent: current.intermittent,
          catch_up_recheck_minutes: 15,
          catch_up_give_up_minutes: 0,
          waiting: [],
        },
      }) as never
    }
    return Promise.resolve({ data: [] }) as never
  })
  vi.mocked(apiClient.post).mockResolvedValue({
    data: { ssh_host_key: current.ssh_host_key },
  } as never)
  vi.mocked(apiClient.delete).mockResolvedValue({} as never)
}

/** Renders the page, then opens `section` from its rail the way a click does. */
async function render(section?: string): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(RepoHostDetailView, {
    props: { id: '5' },
    storeState: { auth: { user: { role: 'admin' } } },
  })
  await flushPromises()
  if (section) {
    const item = wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.text().toLowerCase().startsWith(section))
    if (!item) throw new Error(`no rail item for ${section}`)
    await item.trigger('click')
    await flushPromises()
  }
  return wrapper
}

describe('RepoHostDetailView', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    vi.clearAllMocks()
    setupApi()
  })

  it('names the host and says whether it sleeps', async () => {
    const wrapper = await render()
    const text = wrapper.text()
    expect(text).toContain('nas.lan')
    expect(text).toContain('Not always online')
    expect(text).toContain('2 repositories')
  })

  it('opens on the connection section', async () => {
    const wrapper = await render()
    expect(wrapper.findComponent({ name: 'RepoHostConnectionCard' }).exists()).toBe(true)
  })

  it('holds the power and availability settings together', async () => {
    const wrapper = await render('power')
    expect(wrapper.findComponent({ name: 'RepoHostPowerCard' }).exists()).toBe(true)
    expect(wrapper.findComponent({ name: 'HostAvailabilityCard' }).exists()).toBe(true)
    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts/5/availability')
  })

  it('lists the repositories on the host', async () => {
    const wrapper = await render('repositories')
    expect(wrapper.find('a[href="/repos/1"]').text()).toBe('inhouse-global')
    expect(wrapper.find('a[href="/repos/2"]').text()).toBe('laptops')
  })

  it('refuses to remove a host repositories still use', async () => {
    const wrapper = await render('danger')
    const remove = wrapper.findAll('button').find((b) => b.text() === 'Remove host')!
    expect(remove.attributes('disabled')).toBeDefined()
  })

  it('removes an unused host after confirmation', async () => {
    setupApi(host({ repositories: [] }))
    const wrapper = await render('danger')
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Remove host')!
      .trigger('click')
    await flushPromises()
    dialogButton('Remove').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/repo-hosts/5')
  })

  it('reloads when anything changes', async () => {
    await render()
    vi.mocked(apiClient.get).mockClear()
    wsHandlers.DataChanged?.({})
    await flushPromises()
    expect(apiClient.get).toHaveBeenCalledWith('/repo-hosts/5')
  })

  it('shows why the host could not be loaded', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('repository host 5 not found'))
    const wrapper = await render()
    expect(wrapper.find('.error-banner').text()).toContain('not found')
  })
})
