// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import HelpHint from './HelpHint.vue'

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(HelpHint, {
    props: { label: 'catch up missed runs', ...props },
    slots: { default: 'Missed runs never stack.' },
  })
}

describe('HelpHint', () => {
  it('keeps the explanation hidden until the button is clicked', () => {
    const wrapper = mount()
    expect(wrapper.find('.help-hint-pop').exists()).toBe(false)
    expect(wrapper.find('.help-hint-btn').attributes('aria-expanded')).toBe('false')
  })

  it('names the button for its own field rather than a generic "Help"', () => {
    const wrapper = mount({ label: 'wake host before backup' })
    expect(wrapper.find('.help-hint-btn').attributes('aria-label')).toBe(
      'Help: wake host before backup',
    )
  })

  it('opens on click and shows the slotted explanation', async () => {
    const wrapper = mount()
    await wrapper.find('.help-hint-btn').trigger('click')

    expect(wrapper.find('.help-hint-btn').attributes('aria-expanded')).toBe('true')
    expect(wrapper.find('.help-hint-pop').text()).toBe('Missed runs never stack.')
  })

  it('closes again on a second click', async () => {
    const wrapper = mount()
    await wrapper.find('.help-hint-btn').trigger('click')
    await wrapper.find('.help-hint-btn').trigger('click')

    expect(wrapper.find('.help-hint-pop').exists()).toBe(false)
  })

  it('opens toward the trailing edge when told to', async () => {
    const wrapper = mount({ align: 'end' })
    await wrapper.find('.help-hint-btn').trigger('click')

    expect(wrapper.find('.help-hint-pop').classes()).toContain('help-hint-pop--end')
  })

  it('opens toward the leading edge by default', async () => {
    const wrapper = mount()
    await wrapper.find('.help-hint-btn').trigger('click')

    expect(wrapper.find('.help-hint-pop').classes()).not.toContain('help-hint-pop--end')
  })

  it('renders markup passed to its slot, e.g. a code sample', async () => {
    const wrapper = renderWithPlugins(HelpHint, {
      props: { label: 'exclude patterns' },
      slots: { default: 'Lines starting with <code>#</code> are comments.' },
    })
    await wrapper.find('.help-hint-btn').trigger('click')

    expect(wrapper.find('.help-hint-pop code').text()).toBe('#')
  })
})
