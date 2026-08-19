// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ScheduleTimelineRail, { type TimelineEntry } from './ScheduleTimelineRail.vue'

function inHours(hours: number): string {
  return new Date(Date.now() + hours * 3_600_000).toISOString()
}

function inMs(ms: number): string {
  return new Date(Date.now() + ms).toISOString()
}

function entry(overrides: Partial<TimelineEntry> = {}): TimelineEntry {
  return {
    id: 1,
    label: 'Nightly',
    atIso: inHours(2),
    host: 'borg-backup.example.com',
    ...overrides,
  }
}

describe('ScheduleTimelineRail', () => {
  it('renders nothing when there are no upcoming entries', () => {
    const wrapper = mount(ScheduleTimelineRail, { props: { entries: [] } })
    expect(wrapper.find('.timeline-rail').exists()).toBe(false)
  })

  it('drops entries outside the window and entries already in the past', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2) }),
          entry({ id: 2, atIso: inHours(48) }),
          entry({ id: 3, atIso: inHours(-1) }),
        ],
      },
    })
    expect(wrapper.findAll('.timeline-tick')).toHaveLength(1)
    expect(wrapper.text()).toContain('1 run · 1 host')
  })

  it('does not flag two runs on different hosts as colliding', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'host-a' }),
          entry({ id: 2, atIso: inHours(2.1), host: 'host-b' }),
        ],
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(0)
    expect(wrapper.find('.timeline-note').exists()).toBe(false)
  })

  it('flags two runs on the same host within the collision window', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'shared-host' }),
          entry({ id: 2, atIso: inHours(2.1), host: 'shared-host' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(2)
    expect(wrapper.find('.timeline-note').text()).toContain('2 runs collide on shared-host')
  })

  it('does not flag same-host runs outside the collision window', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'shared-host' }),
          entry({ id: 2, atIso: inHours(4), host: 'shared-host' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(0)
  })

  it('still shows a collision note for two colliding entries whose gap straddles the old rounding-bucket boundary', () => {
    // Regression: grouping used to bucket each entry independently by its
    // own rounded distance from "now" (Math.round(relativeMs / bucketMs)).
    // Two entries a couple of ms apart, straddling a multiple of the
    // 30-minute bucket width, could land in different buckets and vanish
    // from the note despite both rendering as colliding ticks.
    const bucketMs = 30 * 60_000
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inMs(bucketMs / 2 - 1), host: 'shared-host' }),
          entry({ id: 2, atIso: inMs(bucketMs / 2 + 1), host: 'shared-host' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(2)
    expect(wrapper.find('.timeline-note').exists()).toBe(true)
    expect(wrapper.find('.timeline-note').text()).toContain('2 runs collide on shared-host')
  })

  it('groups a transitive chain of collisions into one note, even when the ends do not directly collide', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'shared-host' }),
          entry({ id: 2, atIso: inHours(2.4), host: 'shared-host' }), // 24m after #1: collides with #1
          entry({ id: 3, atIso: inHours(2.8), host: 'shared-host' }), // 24m after #2, 48m after #1
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(3)
    expect(wrapper.find('.timeline-note').text()).toContain('3 runs collide on shared-host')
  })

  it('reports "+N more" when multiple independent collision clusters exist on different hosts', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'host-a' }),
          entry({ id: 2, atIso: inHours(2.1), host: 'host-a' }),
          entry({ id: 3, atIso: inHours(5), host: 'host-b' }),
          entry({ id: 4, atIso: inHours(5.1), host: 'host-b' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(4)
    const note = wrapper.find('.timeline-note').text()
    expect(note).toContain('2 runs collide on host-a')
    expect(note).toContain('(+1 more)')
  })

  it('ignores entries with no host when detecting collisions', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: null }),
          entry({ id: 2, atIso: inHours(2.05), host: null }),
        ],
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(0)
  })
})
