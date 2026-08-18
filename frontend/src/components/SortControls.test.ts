// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import SortControls from './SortControls.vue'

const OPTIONS = [
  { field: 'name', label: 'Name' },
  { field: 'size', label: 'Size' },
] as const

function mountControls(field: 'name' | 'size', direction: 'asc' | 'desc') {
  return mount(SortControls, { props: { field, direction, options: OPTIONS } })
}

describe('SortControls', () => {
  it('renders one button per option', () => {
    const wrapper = mountControls('name', 'asc')
    const labels = wrapper.findAll('button').map((b) => b.text())
    expect(labels).toHaveLength(2)
    expect(labels[0]).toContain('Name')
    expect(labels[1]).toContain('Size')
  })

  it('marks only the active field', () => {
    const wrapper = mountControls('size', 'asc')
    const [name, size] = wrapper.findAll('button')
    expect(name.classes()).not.toContain('active')
    expect(size.classes()).toContain('active')
  })

  it('shows the direction arrow on the active field only', () => {
    const ascending = mountControls('name', 'asc')
    expect(ascending.findAll('button')[0].text()).toContain('↑')
    expect(ascending.findAll('button')[1].text()).not.toContain('↑')

    const descending = mountControls('name', 'desc')
    expect(descending.findAll('button')[0].text()).toContain('↓')
  })

  it('emits toggle with the clicked field', async () => {
    const wrapper = mountControls('name', 'asc')
    await wrapper.findAll('button')[1].trigger('click')
    expect(wrapper.emitted('toggle')).toEqual([['size']])
  })
})
