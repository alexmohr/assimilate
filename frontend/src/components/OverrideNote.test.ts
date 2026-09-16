// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import OverrideNote from './OverrideNote.vue'
import type { ScheduleRow } from '../types/schedule'

function schedule(id: number, name: string | null): ScheduleRow {
  return { id, name } as ScheduleRow
}

function mount(schedules: ScheduleRow[]) {
  return renderWithPlugins(OverrideNote, {
    props: { schedules },
    global: { stubs: { RouterLink: { template: '<a :href="to"><slot /></a>', props: ['to'] } } },
  })
}

describe('OverrideNote', () => {
  it('renders nothing when no schedule overrides the host', () => {
    expect(mount([]).find('.override-note').exists()).toBe(false)
  })

  it('gives each overriding schedule its own row linking to its power section', () => {
    const wrapper = mount([schedule(7, 'Nightly'), schedule(9, 'Weekly offsite')])
    const links = wrapper.findAll('.override-link')
    expect(links.map((l) => l.text())).toEqual(['Nightly', 'Weekly offsite'])
    expect(links[0]?.attributes('href')).toBe('/schedules/7?tab=settings&section=power')
  })

  it('falls back to the schedule number when it has no name', () => {
    expect(
      mount([schedule(12, null)])
        .find('.override-link')
        .text(),
    ).toBe('Schedule #12')
  })

  it('agrees its verb with the count', () => {
    expect(
      mount([schedule(1, 'a')])
        .find('.override-lead')
        .text(),
    ).toContain('1 schedule wakes')
    expect(
      mount([schedule(1, 'a'), schedule(2, 'b')])
        .find('.override-lead')
        .text(),
    ).toContain('2 schedules wake')
  })
})
