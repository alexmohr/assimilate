// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import BaseDisclosure from './BaseDisclosure.vue'

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(BaseDisclosure, {
    props: { title: 'Systemd service unit', ...props },
    slots: { default: '<p class="body">unit contents</p>' },
  })
}

describe('BaseDisclosure', () => {
  it('starts closed and announces that it is', () => {
    const wrapper = mount()
    expect(wrapper.find('.disclosure-head').attributes('aria-expanded')).toBe('false')
    expect(wrapper.find('.disclosure-body').isVisible()).toBe(false)
  })

  it('opens and closes on click, keeping aria-expanded with it', async () => {
    const wrapper = mount()

    await wrapper.find('.disclosure-head').trigger('click')
    expect(wrapper.find('.disclosure-head').attributes('aria-expanded')).toBe('true')
    expect(wrapper.find('.body').isVisible()).toBe(true)

    await wrapper.find('.disclosure-head').trigger('click')
    expect(wrapper.find('.disclosure-head').attributes('aria-expanded')).toBe('false')
    expect(wrapper.find('.disclosure-body').isVisible()).toBe(false)
  })

  it('starts open when the caller says the section is part of the task', () => {
    const wrapper = mount({ defaultOpen: true })
    expect(wrapper.find('.disclosure-head').attributes('aria-expanded')).toBe('true')
    expect(wrapper.find('.body').isVisible()).toBe(true)
  })

  it('says what is inside without being opened', () => {
    const wrapper = mount({ badge: 'Customized', badgeTone: 'warning' })
    const badge = wrapper.find('.disclosure-head .badge')
    expect(badge.text()).toBe('Customized')
    expect(badge.classes()).toContain('badge--warning')
  })

  it('renders no badge when the caller has nothing to say about the contents', () => {
    expect(mount().find('.disclosure-head .badge').exists()).toBe(false)
  })
})
