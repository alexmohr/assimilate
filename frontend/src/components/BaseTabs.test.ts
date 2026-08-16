// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import BaseTabs from './BaseTabs.vue'

const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'archives', label: 'Archives' },
  { id: 'schedules', label: 'Schedules' },
]

function mount(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return renderWithPlugins(BaseTabs, {
    props: { tabs: TABS, modelValue: 'overview', label: 'Repository sections', ...props },
    slots,
  })
}

describe('BaseTabs', () => {
  it('renders one tab per option, in order', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.tab').map((t) => t.text())).toEqual([
      'Overview',
      'Archives',
      'Schedules',
    ])
  })

  it('gives the strip real tab semantics', () => {
    const wrapper = mount()
    const tablist = wrapper.find('[role="tablist"]')
    expect(tablist.attributes('aria-label')).toBe('Repository sections')
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(3)
  })

  it('marks only the selected tab, and leaves the strip a single tab stop', () => {
    const wrapper = mount({ modelValue: 'archives' })
    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs.map((t) => t.attributes('aria-selected'))).toEqual(['false', 'true', 'false'])
    // Roving focus: only the selected tab is reachable with Tab; the arrow
    // keys move within the strip.
    expect(tabs.map((t) => t.attributes('tabindex'))).toEqual(['-1', '0', '-1'])
  })

  it('reports a click as the newly selected tab', async () => {
    const wrapper = mount()
    await wrapper.findAll('.tab')[1].trigger('click')
    expect(wrapper.emitted('update:modelValue')).toEqual([['archives']])
  })

  it('moves selection with the arrow keys and wraps at both ends', async () => {
    const wrapper = mount({ modelValue: 'overview' })
    const tabs = wrapper.findAll('[role="tab"]')

    await tabs[0].trigger('keydown', { key: 'ArrowRight' })
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual(['archives'])

    await tabs[0].trigger('keydown', { key: 'ArrowLeft' })
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual(['schedules'])

    await tabs[2].trigger('keydown', { key: 'ArrowRight' })
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual(['overview'])
  })

  it('jumps to the first and last tab with Home and End', async () => {
    const wrapper = mount({ modelValue: 'archives' })
    const tabs = wrapper.findAll('[role="tab"]')

    await tabs[1].trigger('keydown', { key: 'End' })
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual(['schedules'])

    await tabs[1].trigger('keydown', { key: 'Home' })
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual(['overview'])
  })

  it('ignores keys it does not handle', async () => {
    const wrapper = mount()
    await wrapper.findAll('[role="tab"]')[0].trigger('keydown', { key: 'ArrowDown' })
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })

  it('keeps trailing content out of the tablist, since it is not a tab', () => {
    const wrapper = mount({}, { trailing: '<button class="tab tab-link">Logs</button>' })
    const tablist = wrapper.find('[role="tablist"]')

    // A tablist may only contain tabs. The link shares the row but not the
    // tablist, so assistive tech is not told it is a fourth tab.
    expect(tablist.findAll('.tab')).toHaveLength(3)
    expect(wrapper.find('.tabs').findAll('.tab')).toHaveLength(4)
    expect(tablist.text()).not.toContain('Logs')
    expect(wrapper.text()).toContain('Logs')
  })
})
