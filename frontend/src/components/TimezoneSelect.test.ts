// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import TimezoneSelect from './TimezoneSelect.vue'

describe('TimezoneSelect', () => {
  it('renders the text input', () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: 'UTC' },
    })
    expect(wrapper.find('input[type="text"]').exists()).toBe(true)
  })

  it('displays the current modelValue in the input', () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: 'Europe/Berlin' },
    })
    const input = wrapper.find('input').element as HTMLInputElement
    expect(input.value).toBe('Europe/Berlin')
  })

  it('uses placeholder prop', () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: '', placeholder: 'Pick a timezone' },
    })
    expect(wrapper.find('input').attributes('placeholder')).toBe('Pick a timezone')
  })

  it('opens dropdown on input focus', async () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: 'UTC' },
      attachTo: document.body,
    })
    await wrapper.find('input').trigger('focus')
    expect(wrapper.find('.tz-dropdown').exists()).toBe(true)
    wrapper.unmount()
  })

  it('renders timezone options in dropdown', async () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: 'UTC' },
      attachTo: document.body,
    })
    await wrapper.find('input').trigger('focus')
    const options = wrapper.findAll('.tz-option')
    expect(options.length).toBeGreaterThan(0)
    wrapper.unmount()
  })

  it('emits update:modelValue when an option is clicked', async () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: 'UTC' },
      attachTo: document.body,
    })
    await wrapper.find('input').trigger('focus')
    const firstOption = wrapper.find('.tz-option')
    await firstOption.trigger('mousedown')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
    expect(typeof emitted![0][0]).toBe('string')
    wrapper.unmount()
  })

  it('filters options based on search input', async () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: '' },
      attachTo: document.body,
    })
    await wrapper.find('input').trigger('focus')
    await wrapper.find('input').setValue('Berlin')
    await wrapper.find('input').trigger('input')
    const options = wrapper.findAll('.tz-option')
    expect(options.every((o) => o.text().toLowerCase().includes('berlin'))).toBe(true)
    wrapper.unmount()
  })

  it('shows no results message for unmatched search', async () => {
    const wrapper = mount(TimezoneSelect, {
      props: { modelValue: '' },
      attachTo: document.body,
    })
    await wrapper.find('input').trigger('focus')
    await wrapper.find('input').setValue('zzz-no-match-xyz')
    await wrapper.find('input').trigger('input')
    expect(wrapper.find('.tz-no-results').exists()).toBe(true)
    wrapper.unmount()
  })
})
