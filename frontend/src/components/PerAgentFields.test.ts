// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import PerAgentFields from './PerAgentFields.vue'

const LABELS: Record<number, string> = { 1: 'web-01', 2: 'db-01' }

function mount(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return renderWithPlugins(PerAgentFields, {
    props: {
      agentIds: [1, 2],
      agentLabel: (id: number) => LABELS[id] ?? `#${id}`,
      ...props,
    },
    slots: { default: '<input class="per-agent-input" />', ...slots },
  })
}

describe('PerAgentFields', () => {
  it('renders one labelled block per selected agent', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.per-host-entry')).toHaveLength(2)
    expect(wrapper.findAll('.field-label').map((l) => l.text())).toEqual(['web-01', 'db-01'])
  })

  it('renders the slot content inside each block', () => {
    expect(mount().findAll('.per-agent-input')).toHaveLength(2)
  })

  it('passes the agent id to the slot so the caller can bind per-agent state', () => {
    const wrapper = mount({}, { default: '<span class="id">{{ params.agentId }}</span>' })
    expect(wrapper.findAll('.id').map((s) => s.text())).toEqual(['1', '2'])
  })

  it('renders nothing per-agent when no agents are selected', () => {
    const wrapper = mount({ agentIds: [] })
    expect(wrapper.findAll('.per-host-entry')).toHaveLength(0)
  })

  it('shows the hint only when one is given', () => {
    expect(mount().find('.field-hint').exists()).toBe(false)
    expect(mount({}, { hint: 'Leave empty to inherit' }).find('.field-hint').text()).toBe(
      'Leave empty to inherit',
    )
  })
})
