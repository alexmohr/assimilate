// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ArchiveSelectorRow from './ArchiveSelectorRow.vue'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import { formatClock, formatDate, formatDay } from '../utils/format'

function archive(overrides: Partial<ArchiveEntry> = {}): ArchiveEntry {
  return {
    name: 'web-01-2026-03-01',
    start: '2026-03-01T02:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 2048,
    deduplicated_size: 1024,
    matched: true,
    agent_hostname: 'web-01',
    ...overrides,
  }
}

function row(props: Record<string, unknown> = {}) {
  return mount(ArchiveSelectorRow, {
    props: {
      archive: archive(),
      selected: false,
      showHost: true,
      ...props,
    },
  })
}

function facts(wrapper: ReturnType<typeof row>): string[] {
  return wrapper.findAll('.browser-row-facts > span').map((span) => span.text())
}

describe('ArchiveSelectorRow', () => {
  it('leads with the name and size, then the host, date and dedup size', () => {
    const wrapper = row()
    expect(wrapper.find('.browser-row-title').text()).toBe('web-01-2026-03-01')
    expect(wrapper.find('.browser-row-size').text()).toBe('2.0 KB')
    expect(facts(wrapper)).toEqual(['web-01', '·', '+1.0 KB new'])
    const time = wrapper.find('time')
    expect(time.text()).toBe(formatDay('2026-03-01T02:00:00Z'))
    expect(time.attributes('title')).toBe(formatDate('2026-03-01T02:00:00Z'))
  })

  it('prefers the agent hostname over the one borg recorded', () => {
    const wrapper = row({
      archive: archive({ hostname: 'raw-name', agent_hostname: 'web-01.internal' }),
    })
    expect(facts(wrapper)[0]).toBe('web-01.internal')
  })

  it('shows what borg recorded, not the agent an unmatched archive resembles', () => {
    // An unmatched archive's `agent_hostname` is whichever agent the name
    // happened to look like, so trusting it would name a specific wrong
    // machine and disagree with the group header, which keys unmatched
    // archives by `hostname`.
    const wrapper = row({
      archive: archive({ matched: false, agent_hostname: 'web-01', hostname: 'legacy-nas' }),
    })
    expect(facts(wrapper)[0]).toBe('legacy-nas')
  })

  it('marks an archive no agent claims', () => {
    const wrapper = row({
      archive: archive({ matched: false, agent_hostname: null, hostname: 'legacy-nas' }),
    })
    const host = wrapper.find('.archive-row-unmatched')
    expect(host.exists()).toBe(true)
    expect(host.text()).toBe('legacy-nas')
    expect(row().find('.archive-row-unmatched').exists()).toBe(false)
  })

  it('selects through a real button, so the row is reachable by keyboard', async () => {
    const wrapper = row()
    expect(wrapper.element.tagName).toBe('BUTTON')
    expect(wrapper.attributes('type')).toBe('button')
    expect(wrapper.attributes('aria-label')).toBe('Open archive web-01-2026-03-01')
    await wrapper.trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(1)
  })

  it('selects when the archive name itself is clicked, not only the row edge', async () => {
    const wrapper = row()
    await wrapper.find('.browser-row-title').trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(1)

    await wrapper.find('time').trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(2)
  })

  it('holds no control that acts on the archive, so a stray click can only open it', () => {
    // Deleting, downloading and comparing moved to the open archive's menu.
    const wrapper = row()
    expect(wrapper.element.querySelectorAll('button, a, input')).toHaveLength(0)
  })

  it('names the in-flight delete and dims the row', () => {
    const wrapper = row({ deleting: true })
    expect(wrapper.find('.archive-row-deleting').text()).toBe('Deleting')
    expect(wrapper.classes()).toContain('browser-row--dim')

    const idle = row()
    expect(idle.find('.archive-row-deleting').exists()).toBe(false)
    expect(idle.classes()).not.toContain('browser-row--dim')
  })

  it('shows the time of day under a heading that already names the day', () => {
    const wrapper = row({ clock: true })
    expect(wrapper.find('time').text()).toBe(formatClock('2026-03-01T02:00:00Z'))
    expect(wrapper.find('time').attributes('title')).toBe(formatDate('2026-03-01T02:00:00Z'))
  })

  it('marks the selected row', () => {
    const selected = row({ selected: true })
    expect(selected.classes()).toContain('browser-row--selected')
    expect(selected.attributes('aria-current')).toBe('true')
    const other = row()
    expect(other.classes()).not.toContain('browser-row--selected')
    expect(other.attributes('aria-current')).toBe('false')
  })

  it('tags the row with its repository when the list spans several', () => {
    expect(facts(row({ repoName: 'offsite' }))).toEqual([
      'web-01',
      '·',
      'offsite',
      '·',
      '+1.0 KB new',
    ])
    expect(facts(row())).not.toContain('offsite')
  })

  it('leaves the host out of a list of one machine archives', () => {
    const wrapper = row({ showHost: false })
    expect(facts(wrapper)).toEqual(['+1.0 KB new'])
    expect(wrapper.find('time').exists()).toBe(true)
  })
  describe('queued for maintenance', () => {
    it('steps the row back and says when it goes', () => {
      const wrapper = row({ queued: true })
      expect(wrapper.classes()).toContain('browser-row--dim')
      expect(facts(wrapper)).toContain('Removed at next maintenance')
    })

    it('still only opens the archive: nothing on the row acts on it', async () => {
      const wrapper = row({ queued: true })
      expect(wrapper.find('button[aria-label^="Keep"]').exists()).toBe(false)
      await wrapper.trigger('click')
      expect(wrapper.emitted('select')).toHaveLength(1)
    })

    it('leaves an ordinary archive alone', () => {
      const wrapper = row()
      expect(wrapper.classes()).not.toContain('browser-row--dim')
      expect(facts(wrapper)).not.toContain('Removed at next maintenance')
    })
  })
})
