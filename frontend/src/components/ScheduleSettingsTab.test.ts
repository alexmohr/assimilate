// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { nextTick } from 'vue'
import { renderWithPlugins } from '../test-utils'
import ScheduleSettingsTab from './ScheduleSettingsTab.vue'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleFormState, ScheduleAgentOverrides } from '../types/scheduleForm'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'

/** `power` is present on every real `AgentRow`; the Power section reads it. */
function agentPower(wakeEnabled: boolean) {
  return {
    wake: {
      wake_enabled: wakeEnabled,
      wake_mac_address: wakeEnabled ? '3C:97:0E:2B:9A:44' : null,
      wake_broadcast_address: null,
      wake_timeout_seconds: 180,
      shutdown_after_backup: false,
    },
  }
}

const AGENTS = [
  { id: 10, hostname: 'web-server-01', display_name: 'Web Server', power: agentPower(true) },
  { id: 11, hostname: 'db-server-01', display_name: null, power: agentPower(false) },
] as unknown as AgentRow[]

const REPOS = [
  {
    id: 20,
    name: 'server-daily',
    ssh_host: 'backup-nas.lan',
    power: {
      wake_enabled: false,
      wake_mac_address: null,
      wake_broadcast_address: null,
      wake_timeout_seconds: 180,
      shutdown_after_backup: false,
    },
  },
] as unknown as Repo[]

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
      isBackup: true,
      agents: AGENTS,
      repos: REPOS,
      agentLabel: (id: number) => AGENTS.find((a) => a.id === id)?.display_name ?? `#${id}`,
      form: baseForm(),
      overrides: baseOverrides(),
      selectedAgentIds: [10, 11],
      repoTargets: [{ repo_id: 20, required: true }],
      onFailure: 'stop',
      usePerHostPaths: false,
      perHostSources: {},
      canSeeWakeDetails: true,
      ...props,
    },
  })
}

/** The lead-time field only exists once catch-up is switched on. */
function leadInput(wrapper: ReturnType<typeof mount>) {
  return wrapper.find('#catch-up-lead')
}

function navLabels(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('.settings-nav-item').map((b) => b.text())
}

describe('ScheduleSettingsTab', () => {
  it('shows all five sections for a backup schedule', () => {
    expect(navLabels(mount())).toEqual(['General', 'Targets', 'Power', 'Retention', 'Advanced'])
  })

  // Power stays: a check or verify run needs its hosts reachable just as much
  // as a backup does, and it is the only one of the three that applies to
  // every schedule type.
  it('omits Retention and Advanced for a non-backup schedule', () => {
    expect(navLabels(mount({ isBackup: false }))).toEqual(['General', 'Targets', 'Power'])
  })

  it('renders the Power section, with the wake override and its read-out', () => {
    const wrapper = mount({ section: 'power' })

    expect(wrapper.findAll('.segmented-option').map((o) => o.text())).toEqual([
      'Host default',
      'Enabled',
      'Disabled',
    ])
    expect(wrapper.text()).toContain("Effect on this job's hosts")
  })

  // Drives the choice through the binding the schedule form saves from -
  // rendering the section is not enough on its own to prove it lands there.
  it('writes the wake override back to the form', async () => {
    const form = baseForm()
    const wrapper = mount({ section: 'power', form })

    await wrapper.findAll('.segmented-option')[2]!.trigger('click')

    expect(form.wake_override).toBe('disabled')
  })

  it('emits update:section when a nav item is clicked', async () => {
    const wrapper = mount()
    await wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.text() === 'Targets')!
      .trigger('click')
    expect(wrapper.emitted('update:section')).toEqual([['targets']])
  })

  it('falls back to General when the section prop is unavailable for this schedule type', async () => {
    const wrapper = mount({ isBackup: false, section: 'advanced' })
    expect(wrapper.find('.settings-nav-item[aria-current="true"]').text()).toBe('General')

    await wrapper.find('[aria-label="Help: naming and timing"]').trigger('click')
    expect(wrapper.text()).toContain('What this schedule is called')
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

  /** Type is fixed once a schedule exists; only the creation wizard offers it. */
  it('does not offer the schedule type', () => {
    expect(mount().text()).not.toContain('Schedule type')
  })

  it('marks the repository list as required', () => {
    expect(mount({ section: 'targets' }).findAll('.required')).toHaveLength(1)
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

  it('toggles an agent into selection from the dropdown', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10] })
    await wrapper.find('.multi-select-trigger').trigger('click')
    await wrapper.findAll('.multi-select-item input[type="checkbox"]')[1].trigger('change')

    expect(wrapper.emitted('update:selectedAgentIds')?.at(-1)?.[0]).toEqual([10, 11])
  })

  it('shows Execution Order only with more than one target', () => {
    expect(mount({ section: 'targets', selectedAgentIds: [10, 11] }).text()).toContain(
      'Execution order',
    )
    expect(mount({ section: 'targets', selectedAgentIds: [10] }).text()).not.toContain(
      'Execution order',
    )
  })

  it('reorders targets with the up/down buttons', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11] })
    const downButtons = wrapper.findAll('.order-btn[title="Move down"]')
    await downButtons[0].trigger('click')

    expect(wrapper.emitted('update:selectedAgentIds')?.at(-1)?.[0]).toEqual([11, 10])
  })

  it('moves the second target up', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11] })
    const upButtons = wrapper.findAll('.order-btn[title="Move up"]')
    await upButtons[1].trigger('click')

    expect(wrapper.emitted('update:selectedAgentIds')?.at(-1)?.[0]).toEqual([11, 10])
  })

  it('does nothing when moving the first target up or the last target down', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11] })
    await wrapper.findAll('.order-btn[title="Move up"]')[0].trigger('click')
    await wrapper.findAll('.order-btn[title="Move down"]')[1].trigger('click')

    expect(wrapper.emitted('update:selectedAgentIds')).toBeUndefined()
  })

  it('switches between a shared and a per-agent backup paths editor', async () => {
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11] })
    expect(wrapper.findAll('textarea')).toHaveLength(1)

    const perAgentToggle = wrapper
      .findAllComponents({ name: 'ToggleSwitch' })
      .find((c) => c.props('label') === 'Configure paths per agent')!
    await perAgentToggle.vm.$emit('update:modelValue', true)
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
    expect(wrapper.text()).toContain('Exclude patterns')
    expect(wrapper.text()).toContain('Remote rate limit')
  })

  it('closes the agent dropdown when clicking outside it', async () => {
    const wrapper = mount({ section: 'targets' })
    await wrapper.find('.multi-select-trigger').trigger('click')
    expect(wrapper.find('.multi-select-dropdown').exists()).toBe(true)

    document.body.click()
    await nextTick()

    expect(wrapper.find('.multi-select-dropdown').exists()).toBe(false)
  })

  it('propagates a CronBuilder change into the form', async () => {
    const form = baseForm()
    const wrapper = mount({ form })
    await wrapper.findComponent({ name: 'CronBuilder' }).vm.$emit('update:modelValue', '0 3 * * *')
    expect(form.cron_expression).toBe('0 3 * * *')
  })

  it('toggles Enabled from the General section', async () => {
    const form = baseForm()
    const wrapper = mount({ form })
    await wrapper.find('button[role="switch"]').trigger('click')
    expect(form.enabled).toBe(!DEFAULT_SCHEDULE_FORM_STATE.enabled)
  })

  it('writes into the Missed backup threshold field in the General section', async () => {
    const form = baseForm()
    const wrapper = mount({ form })
    await wrapper.find('input[type="number"]').setValue('5')
    expect(form.missed_backup_threshold).toBe(5)
  })

  it('hides the catch-up floor until catch-up is switched on', () => {
    expect(leadInput(mount({ form: baseForm() })).exists()).toBe(false)
    expect(leadInput(mount({ form: { ...baseForm(), catch_up_missed_runs: true } })).exists()).toBe(
      true,
    )
  })

  it('toggles Catch up missed runs from the General section', async () => {
    const form = baseForm()
    const wrapper = mount({ form })
    const switches = wrapper.findAll('button[role="switch"]')
    await switches[switches.length - 1].trigger('click')
    expect(form.catch_up_missed_runs).toBe(true)
  })

  /** The default 120 minutes reads as 2 hours, not as 120. */
  it('shows a whole-hour floor in hours', async () => {
    const form = { ...baseForm(), catch_up_missed_runs: true }
    const wrapper = mount({ form })
    expect((leadInput(wrapper).element as HTMLInputElement).value).toBe('2')
  })

  it('stores an hour-based floor as minutes', async () => {
    const form = { ...baseForm(), catch_up_missed_runs: true }
    const wrapper = mount({ form })
    await leadInput(wrapper).setValue('3')
    expect(form.catch_up_min_lead_minutes).toBe(180)
  })

  /** A floor that is not a whole number of hours must not be shown as one. */
  it('shows a sub-hour floor in minutes', async () => {
    const form = { ...baseForm(), catch_up_missed_runs: true, catch_up_min_lead_minutes: 45 }
    const wrapper = mount({ form })
    expect((leadInput(wrapper).element as HTMLInputElement).value).toBe('45')
    await leadInput(wrapper).setValue('90')
    expect(form.catch_up_min_lead_minutes).toBe(90)
  })

  it('switches the floor between minutes and hours without changing what is stored', async () => {
    const form = { ...baseForm(), catch_up_missed_runs: true }
    const wrapper = mount({ form })
    const unit = wrapper.find('select[aria-label="Catch-up lead time unit"]')
    await unit.setValue('minutes')
    expect((leadInput(wrapper).element as HTMLInputElement).value).toBe('120')
    expect(form.catch_up_min_lead_minutes).toBe(120)
  })

  /** Zero would let a catch-up double up with the run it is meant to replace. */
  it('never stores a floor below a minute', async () => {
    const form = { ...baseForm(), catch_up_missed_runs: true, catch_up_min_lead_minutes: 30 }
    const wrapper = mount({ form })
    await leadInput(wrapper).setValue('0')
    expect(form.catch_up_min_lead_minutes).toBe(1)
  })

  it('changes the target repository from the Targets section', async () => {
    const repos = [...REPOS, { id: 21, name: 'archive-weekly' }] as unknown as Repo[]
    const wrapper = mount({ section: 'targets', repos })
    await wrapper.findAll('select')[0].setValue('21')
    expect(wrapper.emitted('update:repoTargets')?.at(-1)?.[0]).toEqual([
      { repo_id: 21, required: true },
    ])
  })

  it('adds a second target repository', async () => {
    const repos = [...REPOS, { id: 21, name: 'archive-weekly' }] as unknown as Repo[]
    const wrapper = mount({ section: 'targets', repos })
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Add repository')!
      .trigger('click')
    expect(wrapper.emitted('update:repoTargets')?.at(-1)?.[0]).toEqual([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
  })

  it('changes the On Failure select in the Targets section', async () => {
    const wrapper = mount({ section: 'targets', onFailure: 'stop' })
    await wrapper.findAll('select')[1].setValue('continue')
    expect(wrapper.emitted('update:onFailure')?.at(-1)?.[0]).toBe('continue')
  })

  it('writes into the shared Backup Paths textarea', async () => {
    const form = baseForm()
    const wrapper = mount({ section: 'targets', selectedAgentIds: [10, 11], form })
    await wrapper.find('textarea').setValue('/data\n/etc')
    expect(form.backup_sources).toBe('/data\n/etc')
  })

  it('writes into a per-agent Backup Paths textarea', async () => {
    const perHostSources: Record<number, string> = {}
    const wrapper = mount({
      section: 'targets',
      selectedAgentIds: [10, 11],
      usePerHostPaths: true,
      perHostSources,
    })
    await wrapper.findAll('textarea')[0].setValue('/custom/path')
    expect(perHostSources[10]).toBe('/custom/path')
  })

  it('writes into each retention field', async () => {
    const form = baseForm()
    const wrapper = mount({ section: 'retention', form })
    const inputs = wrapper.findAll('.retention-grid input')
    await inputs[0].setValue('48')
    await inputs[1].setValue('14')
    await inputs[2].setValue('8')
    await inputs[3].setValue('24')
    await inputs[4].setValue('20')

    expect(form.keep_hourly).toBe(48)
    expect(form.keep_daily).toBe(14)
    expect(form.keep_weekly).toBe(8)
    expect(form.keep_monthly).toBe(24)
    expect(form.keep_yearly).toBe(20)
  })

  it('propagates ScheduleAdvancedTab updates for form and overrides', async () => {
    const wrapper = mount({ section: 'advanced' })
    const advancedTab = wrapper.findComponent({ name: 'ScheduleAdvancedTab' })
    expect(advancedTab.exists()).toBe(true)

    const newForm = { ...baseForm(), rate_limit_kbps: 500 }
    const newOverrides = { ...baseOverrides(), usePerHostExcludes: true }
    await advancedTab.vm.$emit('update:form', newForm)
    await advancedTab.vm.$emit('update:overrides', newOverrides)

    expect(wrapper.emitted('update:form')?.at(-1)?.[0]).toEqual(newForm)
    expect(wrapper.emitted('update:overrides')?.at(-1)?.[0]).toEqual(newOverrides)
  })
})
