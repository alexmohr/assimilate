// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import BaseSkeleton from './BaseSkeleton.vue'

describe('BaseSkeleton', () => {
  it('renders skeleton wrapper', () => {
    const wrapper = mount(BaseSkeleton)
    expect(wrapper.find('.skeleton-wrapper').exists()).toBe(true)
  })

  it('renders text variant with one line by default', () => {
    const wrapper = mount(BaseSkeleton, { props: { variant: 'text' } })
    expect(wrapper.findAll('.skeleton-line').length).toBe(1)
  })

  it('renders multiple lines when lines prop is set', () => {
    const wrapper = mount(BaseSkeleton, { props: { variant: 'text', lines: 3 } })
    expect(wrapper.findAll('.skeleton-line').length).toBe(3)
  })

  it('renders card variant', () => {
    const wrapper = mount(BaseSkeleton, { props: { variant: 'card' } })
    expect(wrapper.find('.skeleton-card').exists()).toBe(true)
  })

  it('renders row variant', () => {
    const wrapper = mount(BaseSkeleton, { props: { variant: 'row' } })
    expect(wrapper.find('.skeleton-row').exists()).toBe(true)
  })

  it('renders circle variant', () => {
    const wrapper = mount(BaseSkeleton, { props: { variant: 'circle' } })
    expect(wrapper.find('.skeleton-circle').exists()).toBe(true)
  })

  it('has aria-hidden attribute', () => {
    const wrapper = mount(BaseSkeleton)
    expect(wrapper.find('.skeleton-wrapper').attributes('aria-hidden')).toBe('true')
  })
})
