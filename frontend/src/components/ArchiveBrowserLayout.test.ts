// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'

import ArchiveBrowserLayout from './ArchiveBrowserLayout.vue'

describe('ArchiveBrowserLayout', () => {
  it('renders both slots', () => {
    const wrapper = mount(ArchiveBrowserLayout, {
      slots: {
        list: '<p class="the-list">list</p>',
        browser: '<p class="the-browser">browser</p>',
      },
    })

    expect(wrapper.find('.the-list').exists()).toBe(true)
    expect(wrapper.find('.the-browser').exists()).toBe(true)
  })

  it('splits the width evenly by default', () => {
    const wrapper = mount(ArchiveBrowserLayout)

    expect(wrapper.find('.archive-browser-layout').classes()).not.toContain('layout-narrow-list')
  })

  it('sizes the list pane to a fixed narrow column when narrowList is set', () => {
    const wrapper = mount(ArchiveBrowserLayout, { props: { narrowList: true } })

    expect(wrapper.find('.archive-browser-layout').classes()).toContain('layout-narrow-list')
  })
})
