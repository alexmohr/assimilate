// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { mount } from '@vue/test-utils'
import SettingsRail from './SettingsRail.vue'

type Section = 'general' | 'danger'

function mountRail(section: Section = 'general') {
  return mount(SettingsRail<Section>, {
    props: {
      sections: [
        { id: 'general', label: 'General' },
        { id: 'danger', label: 'Danger zone', danger: true },
      ],
      section,
      label: 'Test settings',
    },
    slots: {
      default: ({ section: s }: { section: Section }) => h('p', { class: 'body' }, `body-${s}`),
    },
  })
}

describe('SettingsRail', () => {
  it('opens the URL section and collapses it again when its head is clicked', async () => {
    const wrapper = mountRail()
    const head = wrapper.find('#acc-head-general')
    expect(head.attributes('aria-expanded')).toBe('true')
    expect(wrapper.find('#acc-body-general').text()).toBe('body-general')

    await head.trigger('click')

    expect(head.attributes('aria-expanded')).toBe('false')
    expect(wrapper.find('#acc-body-general').exists()).toBe(false)
    // Closing a section names no new section for the URL.
    expect(wrapper.emitted('update:section')).toBeUndefined()
  })

  it('opens a closed section and reports it as the current one', async () => {
    const wrapper = mountRail()
    await wrapper.find('#acc-head-danger').trigger('click')

    expect(wrapper.find('#acc-body-danger').text()).toBe('body-danger')
    // Several sections may be open at once.
    expect(wrapper.find('#acc-body-general').exists()).toBe(true)
    expect(wrapper.emitted('update:section')).toEqual([['danger']])
  })
})
