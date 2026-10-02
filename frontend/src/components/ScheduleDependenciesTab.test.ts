// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import {
  getScheduleDependencies,
  listDependencyHosts,
  updateScheduleDependencies,
  type DependencyHost,
} from '../api/dependencyHosts'
import ScheduleDependenciesTab from './ScheduleDependenciesTab.vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleDependencyResponse } from '../types/generated'

vi.mock('../api/dependencyHosts', () => ({
  getScheduleDependencies: vi.fn(),
  listDependencyHosts: vi.fn(),
  updateScheduleDependencies: vi.fn(),
}))

const HOSTS = [
  { id: 5, name: 'nas-media', address: 'nas-media.lan', port: 445 },
  { id: 6, name: 'files-01', address: '10.0.20.14', port: 2049 },
] as unknown as DependencyHost[]

const AGENTS = [
  { id: 10, hostname: 'media-store-01', display_name: null, domain: null },
  { id: 11, hostname: 'web-server-01', display_name: null, domain: 'lan' },
] as unknown as AgentRow[]

const LABELS: Record<number, string> = { 10: 'media-store-01', 11: 'web-server-01' }

/** media-store-01 needs nas-media through its defaults; web-server-01 through this schedule. */
const DEPENDENCIES: ScheduleDependencyResponse[] = [
  {
    agent_id: 10,
    dependency_host_id: 5,
    dependency_name: 'nas-media',
    source: 'agent_default',
    last_check_reachable: false,
  },
  {
    agent_id: 11,
    dependency_host_id: 5,
    dependency_name: 'nas-media',
    source: 'schedule',
    last_check_reachable: false,
  },
]

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleDependenciesTab, {
    props: {
      scheduleId: 1,
      agentIds: [10, 11],
      agents: AGENTS,
      agentLabel: (id: number) => LABELS[id] ?? `#${id}`,
      ...props,
    },
  })
}

function button(wrapper: ReturnType<typeof mount>, label: string) {
  const found = wrapper.findAll('button').find((b) => b.text() === label)
  if (!found) throw new Error(`no "${label}" button`)
  return found
}

describe('ScheduleDependenciesTab', () => {
  beforeEach(() => {
    vi.mocked(listDependencyHosts).mockReset().mockResolvedValue(HOSTS)
    vi.mocked(getScheduleDependencies)
      .mockReset()
      .mockResolvedValue({ dependencies: DEPENDENCIES, waiting: [] })
    vi.mocked(updateScheduleDependencies)
      .mockReset()
      .mockResolvedValue({ dependencies: DEPENDENCIES, waiting: [] })
  })

  it('lists each agent with its dependencies, marking the inherited ones', async () => {
    const wrapper = mount({ agentIds: [10, 11, 12] })
    await flushPromises()

    const dts = wrapper.findAll('.info-grid dt').map((d) => d.text())
    expect(dts).toEqual(['media-store-01', 'web-server-01', '#12'])
    const dds = wrapper.findAll('.info-grid dd')
    const inherited = dds[0].find('.dependency-chip')
    expect(inherited.classes()).toContain('dependency-chip--inherited')
    expect(inherited.text()).toContain('from agent defaults')
    expect(inherited.attributes('href')).toBe('/dependency-hosts/5')
    expect(dds[1].find('.dependency-chip').classes()).not.toContain('dependency-chip--inherited')
    expect(dds[2].text()).toBe('None')
  })

  it('locks the inherited dependency, linking to the agent defaults that set it', async () => {
    const wrapper = mount()
    await flushPromises()
    await button(wrapper, 'Edit').trigger('click')

    const groups = wrapper.findAll('[role="group"]')
    expect(groups).toHaveLength(2)
    const first = groups[0].findAll('input[type="checkbox"]')
    expect((first[0].element as HTMLInputElement).checked).toBe(true)
    expect((first[0].element as HTMLInputElement).disabled).toBe(true)
    expect((first[1].element as HTMLInputElement).checked).toBe(false)
    const link = groups[0].find('.field-hint a')
    expect(link.text()).toBe("media-store-01's backup defaults")
    expect(link.attributes('href')).toBe('/agents/media-store-01?tab=settings&section=defaults')

    const second = groups[1].findAll('input[type="checkbox"]')
    expect((second[0].element as HTMLInputElement).checked).toBe(true)
    expect((second[0].element as HTMLInputElement).disabled).toBe(false)
    expect(groups[1].find('.field-hint a').exists()).toBe(false)
  })

  it('saves only what the schedule sets itself, never the inherited ones', async () => {
    const wrapper = mount()
    await flushPromises()
    await button(wrapper, 'Edit').trigger('click')

    const groups = wrapper.findAll('[role="group"]')
    await groups[0].findAll('input[type="checkbox"]')[1].setValue(true)
    await groups[1].findAll('input[type="checkbox"]')[0].setValue(false)
    await button(wrapper, 'Save').trigger('click')
    await flushPromises()

    expect(updateScheduleDependencies).toHaveBeenCalledWith(1, [
      { agent_id: 10, dependency_host_id: 6 },
    ])
    expect(wrapper.emitted('saved')).toHaveLength(1)
    expect(wrapper.findAll('[role="group"]')).toHaveLength(0)
  })

  it('keeps the form open with the error when the save fails', async () => {
    vi.mocked(updateScheduleDependencies).mockRejectedValue(new Error('nope'))
    const wrapper = mount()
    await flushPromises()
    await button(wrapper, 'Edit').trigger('click')
    await button(wrapper, 'Save').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.findAll('[role="group"]')).toHaveLength(2)
    expect(wrapper.emitted('saved')).toBeUndefined()
  })

  it('points to where dependencies are added when there are none at all', async () => {
    vi.mocked(listDependencyHosts).mockResolvedValue([])
    vi.mocked(getScheduleDependencies).mockResolvedValue({ dependencies: [], waiting: [] })
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.text()).toContain('No dependencies yet.')
    expect(wrapper.find('.field-hint a').attributes('href')).toBe('/agents?tab=dependencies')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('says None for every agent when the schedule needs nothing', async () => {
    vi.mocked(getScheduleDependencies).mockResolvedValue({ dependencies: [], waiting: [] })
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.findAll('.info-grid dd').map((d) => d.text())).toEqual(['None', 'None'])
  })

  it('asks for agents before dependencies on a schedule without any', async () => {
    const wrapper = mount({ agentIds: [] })
    await flushPromises()

    expect(wrapper.text()).toContain('pick them under Targets first')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('reports a failed load', async () => {
    vi.mocked(listDependencyHosts).mockRejectedValue(new Error('down'))
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
  })
})
