// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import EmptyState from './EmptyState.vue'

describe('EmptyState', () => {
  it('renders title prop', () => {
    const wrapper = mount(EmptyState, { props: { title: 'Nothing here' } })
    expect(wrapper.text()).toContain('Nothing here')
  })

  it('renders description when provided', () => {
    const wrapper = mount(EmptyState, {
      props: { title: 'Empty', description: 'Add some items to get started' },
    })
    expect(wrapper.text()).toContain('Add some items to get started')
  })

  it('does not render description element when omitted', () => {
    const wrapper = mount(EmptyState, { props: { title: 'Empty' } })
    expect(wrapper.find('.empty-description').exists()).toBe(false)
  })

  it('renders action button when action prop is provided', () => {
    const wrapper = mount(EmptyState, {
      props: { title: 'Empty', action: 'Create New' },
    })
    expect(wrapper.find('button').text()).toContain('Create New')
  })

  it('does not render action button when action prop is omitted', () => {
    const wrapper = mount(EmptyState, { props: { title: 'Empty' } })
    expect(wrapper.find('button').exists()).toBe(false)
  })

  it('emits action event when action button is clicked', async () => {
    const wrapper = mount(EmptyState, {
      props: { title: 'Empty', action: 'Do It' },
    })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('action')).toBeTruthy()
  })

  it('renders default slot content', () => {
    const wrapper = mount(EmptyState, {
      props: { title: 'Empty' },
      slots: { default: '<span class="custom-slot">Custom</span>' },
    })
    expect(wrapper.find('.custom-slot').exists()).toBe(true)
  })
})
