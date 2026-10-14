// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import {
  mockApiClientRw,
  mockWebSocket,
  resetWsHandlers,
  wsHandlers,
} from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../composables/useWebSocket', () => mockWebSocket())

import { renderWithPlugins } from '../test-utils'
import { machine } from '../test-utils/machineFixtures'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'
import MachinePane from './MachinePane.vue'

const SHARED = machine({
  roles: {
    agent: { id: 1, hostname: 'nas', domain: 'lab' },
    repo_host: { id: 5, ssh_host: 'nas.lan' },
    dependencies: [{ id: 3, name: 'nas-media' }],
  },
})

function mount(props: Record<string, unknown> = {}, slot?: () => ReturnType<typeof h>) {
  return renderWithPlugins(MachinePane, {
    props: { machineId: 20, canEdit: true, ...props },
    slots: slot ? { default: slot } : {},
  })
}

describe('MachinePane', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetWsHandlers()
    vi.mocked(apiClient.get).mockResolvedValue({ data: SHARED } as never)
  })

  it("loads the machine and shows its power and availability, then the role's own sections", async () => {
    const Extra = defineComponent({ name: 'ExtraSection', render: () => h('p', 'agent process') })
    const wrapper = mount({}, () => h(Extra))
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/machines/20')
    expect(wrapper.findComponent({ name: 'MachinePowerSection' }).exists()).toBe(true)
    expect(wrapper.findComponent({ name: 'MachineAvailabilitySection' }).exists()).toBe(true)
    expect(wrapper.text()).toContain('agent process')
  })

  // One machine, several roles: say which, and link to each, so a shared
  // setting is not changed without knowing what else it applies to.
  it('names every role the machine plays, each linking to its own pane', async () => {
    const wrapper = mount()
    await flushPromises()

    const links = wrapper.findAll('.machine-roles a').map((a) => a.attributes('href'))
    expect(links).toEqual([
      '/agents/nas?domain=lab&tab=settings&section=power',
      '/repo-hosts/5?section=power',
      '/dependency-hosts/3?section=power',
    ])
  })

  it('says nothing about sharing for a machine with one role', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: machine() } as never)
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('.machine-roles').exists()).toBe(false)
  })

  it('shows a failed load as an error', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('not found'))
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.get('.error-banner').text()).toContain('not found')
  })

  it('passes a saved machine on, so the page can refresh what it shows of it', async () => {
    const wrapper = mount()
    await flushPromises()
    const saved = machine({ label: 'saved' })
    wrapper.findComponent({ name: 'MachinePowerSection' }).vm.$emit('saved', saved)
    await flushPromises()

    expect(wrapper.emitted('changed')).toEqual([[saved]])
  })

  it('reloads after a check and passes the machine on', async () => {
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()
    wrapper.findComponent({ name: 'MachineAvailabilitySection' }).vm.$emit('checked')
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/machines/20')
    expect(wrapper.emitted('changed')).toHaveLength(1)
  })

  it('reloads when anything changes, and keeps what it shows if that fails', async () => {
    const logged = vi.spyOn(logger, 'error').mockImplementation(() => undefined)
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockRejectedValue(new Error('connection lost'))
    wsHandlers['DataChanged']!({})
    await flushPromises()

    expect(logged).toHaveBeenCalled()
    expect(wrapper.find('.error-banner').exists()).toBe(false)
    expect(wrapper.findComponent({ name: 'MachinePowerSection' }).exists()).toBe(true)
    logged.mockRestore()
  })

  it('loads the other machine when it is pointed at a different one', async () => {
    const wrapper = mount()
    await flushPromises()
    await wrapper.setProps({ machineId: 21 })
    await flushPromises()
    expect(apiClient.get).toHaveBeenLastCalledWith('/machines/21')
  })

  it('explains what sharing a machine means', async () => {
    const wrapper = mount()
    await flushPromises()
    await wrapper.find('[aria-label="Help: settings shared by one machine"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('A change here applies to all of them')
  })
})
