// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import AgentSourcesTab from './AgentSourcesTab.vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'

const AGENT = { id: 3, hostname: 'web-01', is_imported: false } as unknown as AgentRow

const SCHEDULES = [{ id: 9, name: 'Nightly' }] as unknown as ScheduleRow[]

function mountTab(props: Record<string, unknown> = {}) {
  return mount(AgentSourcesTab, {
    props: { agent: AGENT, section: 'vms', isAdmin: true, schedules: SCHEDULES, ...props },
    global: {
      stubs: {
        AgentDefaultsCard: true,
        AgentVmsCard: true,
        AgentDatabasesCard: true,
        AgentSourcesUsedBy: true,
      },
    },
  })
}

function sectionTitles(wrapper: ReturnType<typeof mountTab>): string[] {
  return wrapper.findAll('.accordion-title').map((t) => t.text())
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

  // Files were "Backup defaults" under Settings; they answer the same question
  // as VMs and databases, so they lead the Sources tab.
  it('lists files, virtual machines, databases and used by for an admin', () => {
    expect(sectionTitles(mountTab())).toEqual(['Files', 'Virtual machines', 'Databases', 'Used by'])
  })

  // VMs and databases stay admin-only; files and Used by are for everyone.
  it('hides the admin-only sections from everyone else', () => {
    expect(sectionTitles(mountTab({ isAdmin: false, section: 'files' }))).toEqual([
      'Files',
      'Used by',
    ])
  })

  // `?section=` comes from the URL, so typing an admin-only section's name
  // must not render it for a non-admin.
  it.each([
    ['vms', 'agent-vms-card-stub'],
    ['databases', 'agent-databases-card-stub'],
  ])('does not render the %s pane for a non-admin who asks for it', (section, stub) => {
    const wrapper = mountTab({ isAdmin: false, section })
    expect(wrapper.find(stub).exists()).toBe(false)
    expect(wrapper.find('agent-defaults-card-stub').exists()).toBe(true)
  })

  it('shows the files form, editable unless the host is imported', () => {
    expect(
      mountTab({ section: 'files' }).findComponent({ name: 'AgentDefaultsCard' }).props('canEdit'),
    ).toBe(true)
    expect(
      mountTab({ section: 'files', agent: { ...AGENT, is_imported: true } })
        .findComponent({ name: 'AgentDefaultsCard' })
        .props('canEdit'),
    ).toBe(false)
  })

  // Moved from the Settings tab along with the files form.
  it('keeps the required dependencies off Files', () => {
    expect(
      mountTab({ section: 'files' }).findComponent({ name: 'AgentDependenciesCard' }).exists(),
    ).toBe(false)
  })

  // Moved from the Settings tab along with the files form.
  it('forwards a saved agent from the files form', async () => {
    const wrapper = mountTab({ section: 'files' })
    wrapper.findComponent({ name: 'AgentDefaultsCard' }).vm.$emit('saved', AGENT)
    await wrapper.vm.$nextTick()

    expect(wrapper.emitted('saved')).toEqual([[AGENT]])
  })

  it('hands the schedules to each section that names them', () => {
    expect(
      mountTab({ section: 'vms' }).findComponent({ name: 'AgentVmsCard' }).props('schedules'),
    ).toEqual(SCHEDULES)
    expect(
      mountTab({ section: 'databases' })
        .findComponent({ name: 'AgentDatabasesCard' })
        .props('schedules'),
    ).toEqual(SCHEDULES)
    expect(
      mountTab({ section: 'used-by' })
        .findComponent({ name: 'AgentSourcesUsedBy' })
        .props('schedules'),
    ).toEqual(SCHEDULES)
  })

  it('gives each section an empty schedule list when none were passed', () => {
    const wrapper = mountTab({ section: 'used-by', schedules: undefined })
    expect(wrapper.findComponent({ name: 'AgentSourcesUsedBy' }).props('schedules')).toEqual([])
  })
})
