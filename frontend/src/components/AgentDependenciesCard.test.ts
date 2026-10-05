// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import {
  getAgentDependencies,
  listDependencyHosts,
  updateAgentDependencies,
  type DependencyHost,
} from '../api/dependencyHosts'
import AgentDependenciesCard from './AgentDependenciesCard.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/dependencyHosts', () => ({
  getAgentDependencies: vi.fn(),
  listDependencyHosts: vi.fn(),
  updateAgentDependencies: vi.fn(),
}))

const HOSTS = [
  { id: 5, name: 'nas-media', address: 'nas-media.lan', port: 445 },
  { id: 6, name: 'files-01', address: '10.0.20.14', port: 2049 },
] as unknown as DependencyHost[]

const AGENT = { id: 1, hostname: 'media-store-01', domain: 'lan' } as unknown as AgentRow

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentDependenciesCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
}

function button(wrapper: ReturnType<typeof mount>, label: string) {
  const found = wrapper.findAll('button').find((b) => b.text() === label)
  if (!found) throw new Error(`no "${label}" button`)
  return found
}

describe('AgentDependenciesCard', () => {
  beforeEach(() => {
    vi.mocked(listDependencyHosts).mockReset().mockResolvedValue(HOSTS)
    vi.mocked(getAgentDependencies).mockReset().mockResolvedValue([5])
    vi.mocked(updateAgentDependencies).mockReset().mockResolvedValue([5, 6])
  })

  it('lists what every backup of this agent needs, linked to each', async () => {
    const wrapper = mount()
    await flushPromises()

    expect(getAgentDependencies).toHaveBeenCalledWith('media-store-01', 'lan')
    expect(wrapper.find('.group-label').text()).toBe('Required dependencies')
    const chips = wrapper.findAll('.dependency-chip')
    expect(chips).toHaveLength(1)
    expect(chips[0].text()).toContain('nas-media')
    expect(chips[0].text()).toContain('SMB · nas-media.lan')
    expect(chips[0].attributes('href')).toBe('/dependency-hosts/5')
    expect(wrapper.text()).toContain('Applies to every schedule on this agent.')
  })

  it('says so when nothing is required', async () => {
    vi.mocked(getAgentDependencies).mockResolvedValue([])
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.text()).toContain('None configured.')
  })

  it('saves the ticked dependencies', async () => {
    const wrapper = mount()
    await flushPromises()
    await button(wrapper, 'Edit').trigger('click')

    const boxes = wrapper.findAll('input[type="checkbox"]')
    expect((boxes[0].element as HTMLInputElement).checked).toBe(true)
    expect((boxes[1].element as HTMLInputElement).checked).toBe(false)
    await boxes[1].setValue(true)
    await button(wrapper, 'Save').trigger('click')
    await flushPromises()

    expect(updateAgentDependencies).toHaveBeenCalledWith('media-store-01', [5, 6], 'lan')
    expect(wrapper.findAll('.dependency-chip')).toHaveLength(2)
    expect(wrapper.findAll('input[type="checkbox"]')).toHaveLength(0)
  })

  it('drops an unticked one, and leaves the saved list alone on Cancel', async () => {
    const wrapper = mount()
    await flushPromises()
    await button(wrapper, 'Edit').trigger('click')
    await wrapper.findAll('input[type="checkbox"]')[0].setValue(false)
    await button(wrapper, 'Cancel').trigger('click')

    expect(updateAgentDependencies).not.toHaveBeenCalled()
    expect(wrapper.findAll('.dependency-chip')).toHaveLength(1)
  })

  it('keeps the form open with the error when the save fails', async () => {
    vi.mocked(updateAgentDependencies).mockRejectedValue(new Error('nope'))
    const wrapper = mount()
    await flushPromises()
    await button(wrapper, 'Edit').trigger('click')
    await button(wrapper, 'Save').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.findAll('input[type="checkbox"]')).toHaveLength(2)
  })

  it('offers no Edit to someone who cannot change it', async () => {
    const wrapper = mount({ canEdit: false })
    await flushPromises()

    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
    expect(wrapper.findAll('.dependency-chip')).toHaveLength(1)
  })

  it('points to where dependencies are added when there are none at all', async () => {
    vi.mocked(listDependencyHosts).mockResolvedValue([])
    vi.mocked(getAgentDependencies).mockResolvedValue([])
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.text()).toContain('No dependencies yet.')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('reports a failed load', async () => {
    vi.mocked(getAgentDependencies).mockRejectedValue(new Error('down'))
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
  })
})
