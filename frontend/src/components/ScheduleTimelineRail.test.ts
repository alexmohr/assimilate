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
    repoId: 1,
    repoName: 'server-daily',
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

  it('does not flag two runs on different repositories as colliding', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: 1, repoName: 'repo-a' }),
          entry({ id: 2, atIso: inHours(2.1), repoId: 2, repoName: 'repo-b' }),
        ],
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(0)
    expect(wrapper.find('.timeline-note').exists()).toBe(false)
  })

  it('does not flag two runs sharing a storage host but writing to different repositories', () => {
    // Two repositories on one server do not contend for the same repository
    // lock, so this pair is deliberately not a collision.
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'shared-host', repoId: 1, repoName: 'repo-a' }),
          entry({ id: 2, atIso: inHours(2.1), host: 'shared-host', repoId: 2, repoName: 'repo-b' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(0)
    expect(wrapper.find('.timeline-note').exists()).toBe(false)
  })

  it('flags two runs on the same repository within the collision window', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: 7, repoName: 'shared-repo' }),
          entry({ id: 2, atIso: inHours(2.1), repoId: 7, repoName: 'shared-repo' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(2)
    expect(wrapper.find('.timeline-note').text()).toContain('2 runs collide on shared-repo')
  })

  it('flags a same-repository collision across two different storage hosts of the same repo id', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), host: 'host-a', repoId: 7, repoName: 'shared-repo' }),
          entry({ id: 2, atIso: inHours(2.1), host: 'host-a', repoId: 7, repoName: 'shared-repo' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(2)
  })

  it('does not flag same-repository runs outside the collision window', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: 7, repoName: 'shared-repo' }),
          entry({ id: 2, atIso: inHours(4), repoId: 7, repoName: 'shared-repo' }),
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
          entry({ id: 1, atIso: inMs(bucketMs / 2 - 1), repoId: 7, repoName: 'shared-repo' }),
          entry({ id: 2, atIso: inMs(bucketMs / 2 + 1), repoId: 7, repoName: 'shared-repo' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(2)
    expect(wrapper.find('.timeline-note').exists()).toBe(true)
    expect(wrapper.find('.timeline-note').text()).toContain('2 runs collide on shared-repo')
  })

  it('groups a transitive chain of collisions into one note, even when the ends do not directly collide', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: 7, repoName: 'shared-repo' }),
          // 24m after #1: collides with #1
          entry({ id: 2, atIso: inHours(2.4), repoId: 7, repoName: 'shared-repo' }),
          // 24m after #2, 48m after #1
          entry({ id: 3, atIso: inHours(2.8), repoId: 7, repoName: 'shared-repo' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(3)
    expect(wrapper.find('.timeline-note').text()).toContain('3 runs collide on shared-repo')
  })

  it('reports "+N more" when multiple independent collision clusters exist on different repositories', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: 1, repoName: 'repo-a' }),
          entry({ id: 2, atIso: inHours(2.1), repoId: 1, repoName: 'repo-a' }),
          entry({ id: 3, atIso: inHours(5), repoId: 2, repoName: 'repo-b' }),
          entry({ id: 4, atIso: inHours(5.1), repoId: 2, repoName: 'repo-b' }),
        ],
        collisionMinutes: 30,
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(4)
    const note = wrapper.find('.timeline-note').text()
    expect(note).toContain('2 runs collide on repo-a')
    expect(note).toContain('(+1 more)')
  })

  it('ignores entries with no repository when detecting collisions', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: null, repoName: null }),
          entry({ id: 2, atIso: inHours(2.05), repoId: null, repoName: null }),
        ],
      },
    })
    expect(wrapper.findAll('.timeline-tick-collision')).toHaveLength(0)
  })

  it('names a repository with no known name by its id', () => {
    const wrapper = mount(ScheduleTimelineRail, {
      props: {
        entries: [
          entry({ id: 1, atIso: inHours(2), repoId: 9, repoName: null }),
          entry({ id: 2, atIso: inHours(2.1), repoId: 9, repoName: null }),
        ],
      },
    })
    expect(wrapper.find('.timeline-note').text()).toContain('2 runs collide on repository #9')
  })

  describe('collision list', () => {
    function collidingRail(): ReturnType<typeof mount> {
      return mount(ScheduleTimelineRail, {
        props: {
          entries: [
            entry({ id: 11, label: 'Nightly web', atIso: inHours(2), repoId: 7 }),
            entry({ id: 12, label: 'Nightly db', atIso: inHours(2.1), repoId: 7 }),
            entry({ id: 13, label: 'Elsewhere', atIso: inHours(9), repoId: 8 }),
          ],
          collisionMinutes: 30,
        },
      })
    }

    it('keeps the runs hidden until the note is clicked', () => {
      const wrapper = collidingRail()
      expect(wrapper.find('.timeline-collisions').exists()).toBe(false)
      expect(wrapper.find('.timeline-note').attributes('aria-expanded')).toBe('false')
    })

    it('lists exactly the colliding runs when the note is clicked', async () => {
      const wrapper = collidingRail()
      await wrapper.find('.timeline-note').trigger('click')

      const runs = wrapper.findAll('.timeline-collision-run')
      expect(runs).toHaveLength(2)
      expect(runs[0].text()).toContain('Nightly web')
      expect(runs[1].text()).toContain('Nightly db')
      expect(wrapper.text()).not.toContain('Elsewhere')
      expect(wrapper.find('.timeline-note').attributes('aria-expanded')).toBe('true')
    })

    it('emits the clicked run so the caller can open it', async () => {
      const wrapper = collidingRail()
      await wrapper.find('.timeline-note').trigger('click')
      await wrapper.findAll('.timeline-collision-run')[1].trigger('click')

      const selected = wrapper.emitted('select')
      expect(selected).toHaveLength(1)
      expect((selected?.[0][0] as TimelineEntry).id).toBe(12)
    })

    it('collapses again on a second click', async () => {
      const wrapper = collidingRail()
      await wrapper.find('.timeline-note').trigger('click')
      await wrapper.find('.timeline-note').trigger('click')
      expect(wrapper.find('.timeline-collisions').exists()).toBe(false)
    })
  })
})
