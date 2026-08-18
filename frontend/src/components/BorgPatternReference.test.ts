// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import BorgPatternReference from './BorgPatternReference.vue'

describe('BorgPatternReference', () => {
  it('documents every pattern prefix borg accepts', () => {
    const text = mount(BorgPatternReference).text()
    for (const prefix of ['pp:', 're:', 'fm:']) {
      expect(text).toContain(prefix)
    }
    expect(text).toContain('Shell Patterns (default)')
  })

  it('lists each section with its examples', () => {
    const wrapper = mount(BorgPatternReference)
    expect(wrapper.findAll('.ref-section')).toHaveLength(4)
    expect(wrapper.findAll('.ref-entry').length).toBeGreaterThanOrEqual(8)
  })

  it('defaults to the inline variant', () => {
    const wrapper = mount(BorgPatternReference)
    expect(wrapper.find('.ref-panel').classes()).toContain('ref-panel--inline')
  })

  it('switches to the sidebar variant on request', () => {
    const wrapper = mount(BorgPatternReference, { props: { variant: 'sidebar' } })
    expect(wrapper.find('.ref-panel').classes()).toContain('ref-panel--sidebar')
  })

  it('renders a trailing note only when one is supplied', () => {
    expect(mount(BorgPatternReference).find('.ref-note').exists()).toBe(false)

    const wrapper = mount(BorgPatternReference, {
      slots: { note: 'Schedules can override these.' },
    })
    expect(wrapper.find('.ref-note').text()).toBe('Schedules can override these.')
  })
})
