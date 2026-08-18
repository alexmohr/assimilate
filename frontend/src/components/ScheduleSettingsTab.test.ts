// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleSettingsTab from './ScheduleSettingsTab.vue'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleFormState, ScheduleAgentOverrides } from '../types/scheduleForm'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'

const AGENTS = [
  { id: 10, hostname: 'web-server-01', display_name: 'Web Server' },
  { id: 11, hostname: 'db-server-01', display_name: null },
] as unknown as AgentRow[]

const REPOS = [{ id: 20, name: 'server-daily' }] as unknown as Repo[]

/** A fresh copy, never the shared constant itself - components under test mutate it in place. */
function baseForm(): ScheduleFormState {
  return { ...DEFAULT_SCHEDULE_FORM_STATE }
}

function baseOverrides(): ScheduleAgentOverrides {
  return {
    usePerHostExcludes: false,
    perHostExcludes: {},
    usePerHostFileChangePatterns: false,
    perHostFileChangePatterns: {},
    usePerAgentCmds: false,
    perAgentPreCmds: {},
    perAgentPostCmds: {},
  }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleSettingsTab, {
    props: {
      section: 'general',
      isCreate: false,
      isBackup: true,
      agents: AGENTS,
      repos: REPOS,
      agentLabel: (id: number) => AGENTS.find((a) => a.id === id)?.display_name ?? `#${id}`,
      form: baseForm(),
      overrides: baseOverrides(),
      selectedAgentIds: [10, 11],
      selectedRepoId: 20,
      selectedType: 'backup',
      onFailure: 'stop',
      usePerHostPaths: false,
      perHostSources: {},
      ...props,
    },
  })
}

function navLabels(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('.settings-nav-item').map((b) => b.text())
}

describe('ScheduleSettingsTab', () => {
  it('shows all four sections for a backup schedule', () => {
    expect(navLabels(mount())).toEqual(['General', 'Targets', 'Retention', 'Advanced'])
  })

  it('omits Retention and Advanced for a non-backup schedule', () => {
    expect(navLabels(mount({ isBackup: false }))).toEqual(['General', 'Targets'])
  })

  it('emits update:section when a nav item is clicked', async () => {
    const wrapper = mount()
    await wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.text() === 'Targets')!
      .trigger('click')
    expect(wrapper.emitted('update:section')).toEqual([['targets']])
  })

  it('falls back to General when the section prop is unavailable for this schedule type', () => {
    const wrapper = mount({ isBackup: false, section: 'advanced' })
    expect(wrapper.find('.info-title').text()).toBe('General')
  })

  it('shows the Name field and writes into it', async () => {
    const wrapper = mount()
    const nameInput = wrapper.find('input[placeholder="e.g. Daily web server backup"]')
    await nameInput.setValue('Nightly production backup')
    // `form` is an object-shaped v-model: typing mutates the shared object
    // in place rather than emitting update:form, same as every other field
    // on it (cron, enabled, retention). The input reflecting the new value
    // is what proves the binding round-trips.
    expect((nameInput.element as HTMLInputElement).value).toBe('Nightly production backup')
  })

  it('shows the Schedule Type selector only in create mode', () => {
    expect(mount({ isCreate: true }).text()).toContain('Schedule Type')
    expect(mount({ isCreate: false }).text()).not.toContain('Schedule Type')
  })

  it('shows required markers on Hosts and Repository only in create mode', () => {
    const created = mount({ isCreate: true, section: 'targets' })
    expect(created.findAll('.required')).toHaveLength(2)
    const edited = mount({ isCreate: false, section: 'targets' })
    expect(edited.findAll('.required')).toHaveLength(0)
  })

  it('shows the multi-select summary and opens the dropdown', async () => {
    const wrapper = mount({ section: 'targets' })
    expect(wrapper.find('.multi-select-label').text()).toBe('2 agents selected')

    await wrapper.find('.multi-select-trigger').trigger('click')
    const items = wrapper.findAll('.multi-select-item')
    expect(items.map((i) => i.text())).toEqual(['Web Server', 'db-server-01'])
  })

  it('toggles an agent out of selection from the dropdown', async () => {
    const wrapper = mount({ section: 'targets' })
    await wrapper.find('.multi-select-trigger').trigger('click')
    await wrapper.findAll('.multi-select-item input[type="checkbox"]')[0].trigger('change')

    expect(wrapper.emitted('update:selectedAgentIds')?.at(-1)?.[0]).toEqual([11])
  })

  it('shows Execution Order only with more than one target', () => {
    expect(mount({ section: 'targets', selectedAgentIds: [10, 11] }).text()).toContain(
      'Execution Order',
    )
    expect(mount({ section: 'targets', selectedAgentIds: [10] }).text()).not.toContain(
      'Execution Order',
    )
  })

  it('reorders targets with the up/down buttons', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11] })
    const downButtons = wrapper.findAll('.order-btn[title="Move down"]')
    await downButtons[0].trigger('click')

    expect(wrapper.emitted('update:selectedAgentIds')?.at(-1)?.[0]).toEqual([11, 10])
  })

  it('switches between a shared and a per-agent backup paths editor', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11] })
    expect(wrapper.findAll('textarea')).toHaveLength(1)

    await wrapper.findComponent({ name: 'ToggleSwitch' }).vm.$emit('update:modelValue', true)
    expect(wrapper.emitted('update:usePerHostPaths')?.at(-1)?.[0]).toBe(true)

    await wrapper.setProps({ usePerHostPaths: true })
    expect(wrapper.findAll('textarea')).toHaveLength(2)
  })

  it('hides the retention section body when a different section is active', () => {
    const wrapper = mount({ section: 'retention' })
    expect(wrapper.find('.retention-grid').exists()).toBe(true)
    expect(wrapper.findAll('.retention-grid input').map((i) => i.element.value)).toEqual([
      '24',
      '7',
      '4',
      '12',
      '10',
    ])
  })

  it('renders the Advanced section via ScheduleAdvancedTab', () => {
    const wrapper = mount({ section: 'advanced' })
    expect(wrapper.text()).toContain('Exclude Patterns')
    expect(wrapper.text()).toContain('Remote Rate Limit')
  })
})
