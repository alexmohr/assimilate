// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleAdvancedTab from './ScheduleAdvancedTab.vue'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

function form(overrides: Partial<ScheduleFormState> = {}): ScheduleFormState {
  return {
    name: 'nightly',
    cron_expression: '0 2 * * *',
    enabled: true,
    canary_enabled: false,
    exclude_patterns: '*.cache',
    file_change_patterns: '',
    ignore_global_excludes: false,
    keep_hourly: 0,
    keep_daily: 7,
    keep_weekly: 4,
    keep_monthly: 6,
    keep_yearly: 1,
    compact_enabled: false,
    rate_limit_kbps: 0,
    pre_backup_commands: [],
    post_backup_commands: [],
    hook_timeout_seconds: 60,
    backup_sources: '/srv',
    ...overrides,
  }
}

function agentOverrides(o: Partial<ScheduleAgentOverrides> = {}): ScheduleAgentOverrides {
  return {
    usePerHostExcludes: false,
    perHostExcludes: {},
    usePerHostFileChangePatterns: false,
    perHostFileChangePatterns: {},
    usePerAgentCmds: false,
    perAgentPreCmds: {},
    perAgentPostCmds: {},
    ...o,
  }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleAdvancedTab, {
    props: {
      agentIds: [1, 2],
      agentLabel: (id: number) => `host-0${id}`,
      form: form(),
      overrides: agentOverrides(),
      'onUpdate:form': () => {},
      'onUpdate:overrides': () => {},
      ...props,
    },
  })
}

describe('ScheduleAdvancedTab', () => {
  it('groups the settings into the four labelled sections', () => {
    const titles = mount()
      .findAll('.pane-section > .group-label')
      .map((t) => t.text())
    expect(titles).toEqual(['Options', 'Exclude patterns', 'File change patterns', 'Commands'])
  })

  it('renders the schedule-level values it was given', () => {
    const wrapper = mount()
    const excludes = wrapper.find('textarea.area-input')
    expect((excludes.element as HTMLTextAreaElement).value).toBe('*.cache')
  })

  it('writes an edited rate limit back through the form model', async () => {
    const wrapper = mount()
    const input = wrapper.find('input[type="number"]')
    await input.setValue('512')
    expect(wrapper.props('form')).toMatchObject({ rate_limit_kbps: 512 })
  })

  it('renders the current hook timeout and writes an edit back through the form model', async () => {
    const wrapper = mount({ form: form({ hook_timeout_seconds: 90 }) })
    const inputs = wrapper.findAll('input[type="number"]')
    const timeoutInput = inputs[1]
    expect((timeoutInput.element as HTMLInputElement).value).toBe('90')

    await timeoutInput.setValue('300')
    expect(wrapper.props('form')).toMatchObject({ hook_timeout_seconds: 300 })
  })

  // Each switch is bound to a different form key. Flipping them one at a
  // time catches a v-model pointing at the wrong field, which reads
  // identically in the template and silently saves the wrong setting.
  it.each([
    [0, 'canary_enabled'],
    [1, 'ignore_global_excludes'],
    [2, 'compact_enabled'],
  ])('flips the option switch at %i onto form.%s', async (index, key) => {
    const state = form()
    const wrapper = mount({ form: state })
    const toggles = wrapper.findAllComponents({ name: 'ToggleSwitch' })

    await toggles[index].vm.$emit('update:modelValue', true)

    expect(state[key as keyof typeof state]).toBe(true)
  })

  it.each([
    ['Exclude Patterns', 'usePerHostExcludes'],
    ['File Change Patterns', 'usePerHostFileChangePatterns'],
    ['Commands', 'usePerAgentCmds'],
  ])('flips the %s per-agent switch onto overrides.%s', async (_section, key) => {
    const state = agentOverrides()
    const wrapper = mount({ overrides: state })
    const perAgentToggles = wrapper
      .findAll('.field-inline')
      .filter((f) => f.text().includes('per agent'))

    for (const field of perAgentToggles) {
      await field.findComponent({ name: 'ToggleSwitch' }).vm.$emit('update:modelValue', true)
    }

    expect(state[key as keyof typeof state]).toBe(true)
  })

  it('writes edited exclude patterns back through the form model', async () => {
    const state = form()
    const wrapper = mount({ form: state })
    await wrapper.find('textarea.area-input').setValue('pp:__pycache__')
    expect(state.exclude_patterns).toBe('pp:__pycache__')
  })

  it('writes edited file change patterns back through the form model', async () => {
    const state = form()
    const wrapper = mount({ form: state })
    await wrapper
      .findComponent({ name: 'FileChangePatternsEditor' })
      .vm.$emit('update:modelValue', '**/*.log')
    expect(state.file_change_patterns).toBe('**/*.log')
  })

  // The per-agent switches only make sense on a multi-host schedule; with a
  // single agent there is nothing to vary, so the toggle must not appear.
  it('hides the per-agent switches for a single-agent schedule', () => {
    const wrapper = mount({ agentIds: [1] })
    expect(wrapper.findAll('.field-inline').map((f) => f.text())).not.toContain(
      expect.stringContaining('Configure per agent'),
    )
    expect(wrapper.text()).not.toContain('Configure per agent')
  })

  it('offers a per-agent switch per section on a multi-agent schedule', () => {
    const wrapper = mount()
    const perAgent = wrapper.findAll('.field-inline').filter((f) => f.text().includes('per agent'))
    expect(perAgent).toHaveLength(3)
  })

  describe('exclude patterns', () => {
    it('edits one shared list while per-agent excludes are off', () => {
      const wrapper = mount()
      expect(wrapper.find('textarea.area-input').exists()).toBe(true)
      expect(wrapper.findComponent({ name: 'PerAgentFields' }).exists()).toBe(false)
    })

    it('swaps to a field per agent once per-agent excludes are on', () => {
      const wrapper = mount({ overrides: agentOverrides({ usePerHostExcludes: true }) })
      expect(wrapper.findComponent({ name: 'PerAgentFields' }).exists()).toBe(true)
      expect(wrapper.text()).toContain('host-01')
      expect(wrapper.text()).toContain('host-02')
    })

    it('shows the shared hint only in shared mode, so it cannot contradict the fields', () => {
      expect(mount().text()).toContain('Leave empty to use only global')
      const perAgent = mount({ overrides: agentOverrides({ usePerHostExcludes: true }) })
      expect(perAgent.text()).toContain('Leave an agent empty')
    })

    it('routes an edited per-agent pattern to that agent', async () => {
      const overrides = agentOverrides({ usePerHostExcludes: true })
      const wrapper = mount({ overrides })
      await wrapper.findAll('textarea')[1].setValue('/var/tmp')
      expect(overrides.perHostExcludes[2]).toBe('/var/tmp')
    })
  })

  describe('pattern reference', () => {
    it('stays closed until asked for', () => {
      const wrapper = mount()
      expect(wrapper.findComponent({ name: 'BorgPatternReference' }).exists()).toBe(false)
      expect(wrapper.find('.ref-toggle').text()).toBe('Pattern Reference')
    })

    it('opens and closes on the toggle, relabelling itself', async () => {
      const wrapper = mount()
      await wrapper.find('.ref-toggle').trigger('click')
      expect(wrapper.findComponent({ name: 'BorgPatternReference' }).exists()).toBe(true)
      expect(wrapper.find('.ref-toggle').text()).toBe('Close Reference')

      await wrapper.find('.ref-toggle').trigger('click')
      expect(wrapper.findComponent({ name: 'BorgPatternReference' }).exists()).toBe(false)
    })
  })

  describe('file change patterns', () => {
    it('uses the dedicated editor in shared mode', () => {
      const wrapper = mount()
      expect(wrapper.findComponent({ name: 'FileChangePatternsEditor' }).exists()).toBe(true)
    })

    it('swaps to plain per-agent fields once overridden', () => {
      const wrapper = mount({
        overrides: agentOverrides({ usePerHostFileChangePatterns: true }),
      })
      expect(wrapper.findComponent({ name: 'FileChangePatternsEditor' }).exists()).toBe(false)
      expect(wrapper.findComponent({ name: 'PerAgentFields' }).exists()).toBe(true)
    })

    it('routes an edited per-agent pattern to that agent', async () => {
      const overrides = agentOverrides({ usePerHostFileChangePatterns: true })
      const wrapper = mount({ overrides })
      const fields = wrapper.findAll('textarea[placeholder="File change patterns, one per line"]')
      await fields[0].setValue('**/*.sql')
      expect(overrides.perHostFileChangePatterns[1]).toBe('**/*.sql')
    })
  })

  describe('commands', () => {
    it('offers one pre and one post command list editor in shared mode', () => {
      const wrapper = mount()
      expect(wrapper.findAllComponents({ name: 'CommandListEditor' })).toHaveLength(2)
    })

    // Per-agent mode keeps the same editor but gives each agent its own pre
    // and post pair, so two agents means four command list editors.
    it('swaps both to a pre/post pair per agent once overridden', () => {
      const wrapper = mount({ overrides: agentOverrides({ usePerAgentCmds: true }) })
      expect(wrapper.findAllComponents({ name: 'CommandListEditor' })).toHaveLength(4)
      expect(wrapper.findComponent({ name: 'PerAgentFields' }).exists()).toBe(true)
      expect(wrapper.findAll('.form-sublabel').map((l) => l.text())).toEqual([
        'Pre-backup',
        'Post-backup',
        'Pre-backup',
        'Post-backup',
      ])
    })

    it('routes an edited per-agent command to that agent and phase', async () => {
      const overrides = agentOverrides({ usePerAgentCmds: true })
      const wrapper = mount({ overrides })
      const editors = wrapper.findAllComponents({ name: 'CommandListEditor' })

      await editors[0].vm.$emit('update:modelValue', ['pre on host-01'])
      await editors[3].vm.$emit('update:modelValue', ['post on host-02'])

      expect(overrides.perAgentPreCmds[1]).toEqual(['pre on host-01'])
      expect(overrides.perAgentPostCmds[2]).toEqual(['post on host-02'])
    })

    it('writes edited commands back through the form model', async () => {
      const wrapper = mount()
      await wrapper
        .findAllComponents({ name: 'CommandListEditor' })[0]
        .vm.$emit('update:modelValue', ['systemctl stop app'])
      expect(wrapper.props('form')).toMatchObject({ pre_backup_commands: ['systemctl stop app'] })
    })

    it('writes edited post-backup commands back through the form model', async () => {
      const state = form()
      const wrapper = mount({ form: state })
      await wrapper
        .findAllComponents({ name: 'CommandListEditor' })[1]
        .vm.$emit('update:modelValue', ['systemctl start app'])
      expect(state.post_backup_commands).toEqual(['systemctl start app'])
    })

    it('gives each command list editor an accessible name, in both shared and per-agent mode', () => {
      const shared = mount().findAllComponents({ name: 'CommandListEditor' })
      expect(shared.map((e) => e.props('ariaLabel'))).toEqual([
        'Pre-backup commands',
        'Post-backup commands',
      ])

      const perAgent = mount({
        overrides: agentOverrides({ usePerAgentCmds: true }),
      }).findAllComponents({ name: 'CommandListEditor' })
      expect(perAgent.map((e) => e.props('ariaLabel'))).toEqual([
        'Pre-backup commands',
        'Post-backup commands',
        'Pre-backup commands',
        'Post-backup commands',
      ])
    })
  })
})
