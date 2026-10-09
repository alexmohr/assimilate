// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScopeSelector, { type ScopeOption } from './ScopeSelector.vue'
import type { ChannelScope } from '../types/generated'

const REPOS: ScopeOption[] = [
  { id: 1, label: 'server-daily' },
  { id: 2, label: 'media-weekly' },
]
const AGENTS: ScopeOption[] = [{ id: 3, label: 'web-01' }]
const SCHEDULES: ScopeOption[] = [{ id: 4, label: 'nightly' }]

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScopeSelector, {
    props: {
      selected: {} as ChannelScope,
      repos: REPOS,
      agents: AGENTS,
      schedules: SCHEDULES,
      search: '',
      ...props,
    },
  })
}

describe('ScopeSelector', () => {
  it('lists every option under its section', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.scope-section-title').map((t) => t.text())).toEqual([
      'Repositories',
      'Hosts',
      'Schedules',
    ])
    expect(wrapper.findAll('.scope-item').map((i) => i.text())).toEqual([
      'server-daily',
      'media-weekly',
      'web-01',
      'nightly',
    ])
  })

  it('omits a section that has no options at all', () => {
    const wrapper = mount({ agents: [] })
    expect(wrapper.findAll('.scope-section-title').map((t) => t.text())).toEqual([
      'Repositories',
      'Schedules',
    ])
  })

  it('ticks the ids already in the scope', () => {
    const wrapper = mount({ selected: { repo_ids: [2], schedule_ids: [4] } })
    const checked = wrapper
      .findAll<HTMLInputElement>('.scope-item input[type="checkbox"]')
      .map((c) => c.element.checked)
    expect(checked).toEqual([false, true, false, true])
  })

  it('filters by the search text, ignoring case', () => {
    const wrapper = mount({ search: '  MEDIA ' })
    expect(wrapper.findAll('.scope-item').map((i) => i.text())).toEqual(['media-weekly'])
  })

  it('reports search edits through v-model', async () => {
    const wrapper = mount()
    await wrapper.find('.scope-search').setValue('web')
    expect(wrapper.emitted('update:search')).toEqual([['web']])
  })

  it('emits the section and id of a toggled option', async () => {
    const wrapper = mount()
    const boxes = wrapper.findAll('.scope-item input[type="checkbox"]')
    await boxes[2].trigger('change')
    await boxes[3].trigger('change')
    expect(wrapper.emitted('toggle')).toEqual([
      ['agent_ids', 3],
      ['schedule_ids', 4],
    ])
  })
})
