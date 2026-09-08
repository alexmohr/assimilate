// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { nextTick } from 'vue'
import { renderWithPlugins } from '../test-utils'
import AgentMultiSelect from './AgentMultiSelect.vue'
import type { AgentRow } from '../types/agent'

const AGENTS = [
  { id: 10, hostname: 'web-server-01', display_name: 'Web Server' },
  { id: 11, hostname: 'db-server-01', display_name: null },
] as unknown as AgentRow[]

function mount(modelValue: number[], props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentMultiSelect, { props: { agents: AGENTS, modelValue, ...props } })
}

describe('AgentMultiSelect', () => {
  /** The inline dropdown this replaced named the host when exactly one was
      picked; "1 agent selected" says nothing the operator did not already
      know, and one host is the common case. */
  it('summarises the selection, naming a single host', () => {
    expect(mount([]).find('.multi-select-label').text()).toBe('Select agents...')
    expect(mount([10]).find('.multi-select-label').text()).toBe('Web Server')
    expect(mount([10, 11]).find('.multi-select-label').text()).toBe('2 agents selected')
  })

  it('names a single host by hostname when it has no display name', () => {
    expect(mount([11]).find('.multi-select-label').text()).toBe('db-server-01')
  })

  it('falls back to the hostname when an agent has no display name', async () => {
    const wrapper = mount([])
    await wrapper.find('.multi-select-trigger').trigger('click')
    expect(wrapper.findAll('.multi-select-item').map((i) => i.text())).toEqual([
      'Web Server',
      'db-server-01',
    ])
  })

  it('adds and removes an agent', async () => {
    const wrapper = mount([10])
    await wrapper.find('.multi-select-trigger').trigger('click')
    const boxes = wrapper.findAll('.multi-select-item input[type="checkbox"]')

    await boxes[1].trigger('change')
    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toEqual([10, 11])

    // `defineModel` keeps its own value when the parent does not write the
    // prop back, so this deselects 10 from the list the first click produced.
    await boxes[0].trigger('change')
    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toEqual([11])
  })

  it('closes on a click outside', async () => {
    const wrapper = mount([])
    await wrapper.find('.multi-select-trigger').trigger('click')
    expect(wrapper.find('.multi-select-dropdown').exists()).toBe(true)

    document.body.click()
    await nextTick()

    expect(wrapper.find('.multi-select-dropdown').exists()).toBe(false)
  })

  it('cannot be opened while disabled', () => {
    const wrapper = mount([], { disabled: true })
    expect(wrapper.find('.multi-select-trigger').attributes('disabled')).toBeDefined()
  })
})
