// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleCard from './ScheduleCard.vue'
import type { ScheduleRow } from '../types/schedule'

const SCHEDULE = {
  id: 7,
  name: 'nightly',
  cron_expression: '0 2 * * *',
  enabled: true,
  schedule_type: 'backup',
  target_hostnames: ['web-01'],
  next_run_at: '2026-02-01T02:00:00Z',
  last_run_at: '2026-01-31T02:00:00Z',
} as unknown as ScheduleRow

function mount(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return renderWithPlugins(ScheduleCard, {
    props: {
      schedule: SCHEDULE,
      issues: [],
      formatRun: (v: string | null) => v ?? 'never',
      ...props,
    },
    slots,
  })
}

describe('ScheduleCard', () => {
  it('renders the schedule name, cadence and both run times', () => {
    const wrapper = mount()
    expect(wrapper.text()).toContain('nightly')
    expect(wrapper.text()).toContain('2026-02-01T02:00:00Z')
    expect(wrapper.text()).toContain('2026-01-31T02:00:00Z')
  })

  it('labels the schedule type through the shared badge', () => {
    expect(mount().find('.badge--neutral').text()).toBe('Backup')
    expect(
      mount({ schedule: { ...SCHEDULE, schedule_type: 'check' } })
        .find('.badge--neutral')
        .text(),
    ).toBe('Integrity Check')
    expect(
      mount({ schedule: { ...SCHEDULE, schedule_type: 'verify' } })
        .find('.badge--neutral')
        .text(),
    ).toBe('Verify (extract dry-run)')
  })

  it('marks a disabled schedule', () => {
    const wrapper = mount({ schedule: { ...SCHEDULE, enabled: false } })
    expect(wrapper.find('.entity-card').classes()).toContain('entity-card--notable')
    expect(wrapper.text()).toContain('Disabled')
  })

  it('highlights only when the caller asks', () => {
    expect(mount().find('.entity-card').classes()).not.toContain('entity-card--highlighted')
    expect(mount({ highlighted: true }).find('.entity-card').classes()).toContain(
      'entity-card--highlighted',
    )
  })

  it('emits select when the card is clicked', async () => {
    const wrapper = mount()
    await wrapper.find('.entity-card').trigger('click')
    expect(wrapper.emitted('select')).toBeTruthy()
  })

  it('does not emit select when an action inside the card is clicked', async () => {
    const wrapper = mount({}, { actions: '<button class="run">Run</button>' })
    await wrapper.find('button.run').trigger('click')
    expect(wrapper.emitted('select')).toBeFalsy()
  })

  it('omits the action row when no actions are given', () => {
    expect(mount().find('.card-actions').exists()).toBe(false)
    expect(mount({}, { actions: '<button>Run</button>' }).find('.card-actions').exists()).toBe(true)
  })

  it('lets the caller override the title and add meta', () => {
    const wrapper = mount(
      {},
      { title: 'custom title', meta: '<span class="hosts">2 agents</span>' },
    )
    expect(wrapper.find('.card-name').text()).toBe('custom title')
    expect(wrapper.find('.hosts').text()).toBe('2 agents')
  })
})
