// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleAdvancedTab from './ScheduleAdvancedTab.vue'
import type { AgentRow } from '../types/agent'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleFormState } from '../types/scheduleForm'

function form(overrides: Partial<ScheduleFormState> = {}): ScheduleFormState {
  return {
    ...DEFAULT_SCHEDULE_FORM_STATE,
    name: 'nightly',
    canary_enabled: false,
    exclude_patterns: '*.cache',
    keep_hourly: 0,
    keep_monthly: 6,
    keep_yearly: 1,
    compact_enabled: false,
    backup_sources: '/srv',
    ...overrides,
  }
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleAdvancedTab, {
    props: {
      agentIds: [1, 2],
      agentLabel: (id: number) => `host-0${id}`,
      form: form(),
      'onUpdate:form': () => {},
      ...props,
    },
  })
}

describe('ScheduleAdvancedTab', () => {
  it('says in the pane head what the tab covers, not behind a help icon', () => {
    const wrapper = mount()
    expect(wrapper.find('.pane-head .pane-lede').text()).toContain(
      'Settings most schedules leave alone: bandwidth, verification',
    )
    expect(wrapper.find('[aria-label="Help: bandwidth, patterns and commands"]').exists()).toBe(
      false,
    )
  })

  // What a schedule backs up - its paths, excludes, file change patterns and
  // commands, and the VM and database switches - moved to Settings › Sources
  // (ScheduleSourcesPane). Advanced keeps only the options section.
  it('groups the settings into the one labelled options section', () => {
    const titles = mount()
      .findAll('.pane-section > .group-label')
      .map((t) => t.text())
    expect(titles).toEqual(['Options'])
  })

  it('points to Sources for what the schedule backs up', () => {
    expect(mount().find('.pane-head .pane-lede').text()).toContain(
      'What a schedule backs up, with its excludes and commands, is under Sources.',
    )
  })

  it('no longer edits patterns, commands or the VM and database switches', () => {
    const wrapper = mount()
    expect(wrapper.find('textarea').exists()).toBe(false)
    expect(wrapper.findComponent({ name: 'FileChangePatternsEditor' }).exists()).toBe(false)
    expect(wrapper.findComponent({ name: 'CommandListEditor' }).exists()).toBe(false)
    expect(wrapper.findComponent({ name: 'PerAgentFields' }).exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Back up virtual machines')
    expect(wrapper.text()).not.toContain('Dump databases')
  })

  // Every setting is one row: the name and its description on the left, the
  // control on the right. A control that escapes `.pane-row-control` goes
  // back to floating at the far edge of a wide pane, which is what the row
  // shape exists to stop.
  it('pairs every setting with its control inside one row', () => {
    const rows = mount().findAll('.pane-row')
    expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) {
      expect(row.find('.pane-row-control').exists()).toBe(true)
    }
  })

  it('describes each option under its own name rather than between the rows', () => {
    const options = mount().findAll('.pane-section')[0].findAll('.pane-row')
    const described = options.map((row) => ({
      title: row.find('.field-title').text(),
      // Most explanations now sit behind a `HelpHint`, disclosed on click
      // rather than printed permanently under the name; the rate limit row
      // keeps a visible `.field-hint` too, for the "set to 0" value itself.
      hint:
        row.find('.field-body .help-hint').exists() ||
        row.find('.field-body > .field-hint').exists(),
    }))
    expect(described).toEqual([
      { title: 'Canary verification', hint: true },
      { title: 'Ignore global excludes', hint: true },
      { title: 'Compact after backup', hint: true },
      { title: 'Filesystem snapshots', hint: true },
      { title: 'Remote rate limit (kB/s)', hint: true },
      { title: 'Hook command timeout (seconds)', hint: true },
    ])
  })

  describe('option hints', () => {
    it('discloses the canary hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: catching a silent failure"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        'Writes a canary file before the backup',
      )
    })

    it('discloses the ignore-global-excludes hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: skipping the server-wide list"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        "Back up using only this schedule's patterns",
      )
    })

    it('discloses the compact hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: reclaiming freed space"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        'Runs borg compact once pruning is done',
      )
    })

    it('discloses the rate limit hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: capping upload bandwidth"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toBe("Caps borg's upload bandwidth.")
    })

    it('discloses the hook timeout hint with its explainer text', async () => {
      const wrapper = mount()
      await wrapper.find('[aria-label="Help: the default per-command budget"]').trigger('click')
      expect(wrapper.find('.help-hint-pop').text()).toContain(
        'The default for every pre- and post-backup command that does not set its own.',
      )
    })
  })

  it('writes an edited rate limit back through the form model', async () => {
    const wrapper = mount()
    const input = wrapper.find('#schedule-rate-limit')
    await input.setValue('512')
    expect(wrapper.props('form')).toMatchObject({ rate_limit_kbps: 512 })
  })

  it('renders the current hook timeout and writes an edit back through the form model', async () => {
    const wrapper = mount({ form: form({ hook_timeout_seconds: 90 }) })
    const timeoutInput = wrapper.find('#schedule-hook-timeout')
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
    expect(toggles).toHaveLength(3)

    await toggles[index].vm.$emit('update:modelValue', true)

    expect(state[key as keyof typeof state]).toBe(true)
  })
})

describe('ScheduleAdvancedTab filesystem snapshots', () => {
  const agent = (id: number, is_connected: boolean, snapshot_tools: string[]) =>
    ({ id, is_connected, snapshot_tools }) as unknown as AgentRow

  it('writes the chosen mode back through the form model', async () => {
    const state = form({ fs_snapshot: 'off' })
    const wrapper = mount({ form: state })
    const options = wrapper.findAll('[role="radiogroup"][aria-label="Filesystem snapshots"] button')
    expect(options.map((option) => option.text())).toEqual(['Off', 'Required', 'If possible'])
    await options[2]!.trigger('click')
    expect(state.fs_snapshot).toBe('best_effort')
  })

  it('asks for the LVM snapshot size only while snapshots are on', async () => {
    expect(
      mount({ form: form({ fs_snapshot: 'off' }) })
        .find('#schedule-lvm-snapshot-percent')
        .exists(),
    ).toBe(false)
    const state = form({ fs_snapshot: 'required', lvm_snapshot_percent: 10 })
    const wrapper = mount({ form: state })
    const input = wrapper.find('#schedule-lvm-snapshot-percent')
    expect((input.element as HTMLInputElement).value).toBe('10')
    await input.setValue('30')
    expect(state.lvm_snapshot_percent).toBe(30)
  })

  it.each([
    ['Filesystem snapshots', 'If possible'],
    ['LVM snapshot size (%)', 'so size it for the writes'],
  ])('explains %s on request', async (title, explanation) => {
    const row = mount({ form: form({ fs_snapshot: 'required' }) })
      .findAll('.pane-row')
      .find((candidate) => candidate.find('.field-title').text() === title)!
    await row.find('.help-hint-btn').trigger('click')
    expect(row.find('.help-hint-pop').text()).toContain(explanation)
  })

  it('names the connected hosts that found no snapshot tool', () => {
    const agents = [
      agent(1, true, []),
      agent(2, true, ['zfs']),
      agent(3, false, []),
      agent(4, true, []),
    ]
    const hint = (mode: ScheduleFormState['fs_snapshot']) =>
      mount({ form: form({ fs_snapshot: mode }), agentIds: [1, 2, 3], agents })
        .findAll('.pane-row')
        .find((row) => row.find('.field-title').text() === 'Filesystem snapshots')!
        .find('.field-hint')
    // host-04 is not on this schedule, host-03 is offline and has said nothing.
    expect(hint('best_effort').text()).toBe(
      'No zfs, btrfs or LVM tools reported by host-01. An agent older than this server reports none: update it if the tools are installed.',
    )
    expect(hint('off').exists()).toBe(false)
  })
})
