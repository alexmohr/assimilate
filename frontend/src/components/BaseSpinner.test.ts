// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import BaseSpinner from './BaseSpinner.vue'

describe('BaseSpinner', () => {
  it('renders spinner element with status role', () => {
    const wrapper = mount(BaseSpinner)
    const status = wrapper.find('[role="status"]')
    expect(status.exists()).toBe(true)
  })

  it('renders SVG spinner icon', () => {
    const wrapper = mount(BaseSpinner)
    expect(wrapper.find('svg.spinner-icon').exists()).toBe(true)
  })

  it('uses default label "Loading"', () => {
    const wrapper = mount(BaseSpinner)
    expect(wrapper.find('[role="status"]').attributes('aria-label')).toBe('Loading')
  })

  it('accepts custom label prop', () => {
    const wrapper = mount(BaseSpinner, { props: { label: 'Please wait' } })
    expect(wrapper.find('[role="status"]').attributes('aria-label')).toBe('Please wait')
  })

  it('applies size class based on size prop', () => {
    const wrapper = mount(BaseSpinner, { props: { size: 'lg' } })
    expect(wrapper.find('svg').classes()).toContain('spinner-lg')
  })

  it('renders slot text when provided', () => {
    const wrapper = mount(BaseSpinner, {
      slots: { default: 'Loading data...' },
    })
    expect(wrapper.find('.spinner-text').exists()).toBe(true)
    expect(wrapper.text()).toContain('Loading data...')
  })

  it('does not render spinner-text when no slot provided', () => {
    const wrapper = mount(BaseSpinner)
    expect(wrapper.find('.spinner-text').exists()).toBe(false)
  })
})
