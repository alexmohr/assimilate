// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import AgentSourcesTab from './AgentSourcesTab.vue'
import type { AgentRow } from '../types/agent'

const AGENT = { id: 3, hostname: 'web-01', is_imported: false } as unknown as AgentRow

function mountTab() {
  return mount(AgentSourcesTab, {
    props: { agent: AGENT, section: 'vms' },
    global: { stubs: { AgentVmsCard: true, AgentDatabasesCard: true } },
  })
}

describe('AgentSourcesTab', () => {
  it('shows the requested section and passes newly opened sections up for the URL', async () => {
    const wrapper = mountTab()
    expect(wrapper.find('agent-vms-card-stub').exists()).toBe(true)
    expect(wrapper.find('agent-databases-card-stub').exists()).toBe(false)

    await wrapper.find('#acc-head-databases').trigger('click')

    expect(wrapper.find('agent-databases-card-stub').exists()).toBe(true)
    expect(wrapper.emitted('update:section')).toEqual([['databases']])
  })
})
