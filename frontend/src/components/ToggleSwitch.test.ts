// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ToggleSwitch from './ToggleSwitch.vue'

describe('ToggleSwitch', () => {
  it('renders toggle button with switch role', () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false } })
    expect(wrapper.find('[role="switch"]').exists()).toBe(true)
  })

  it('reflects off state via aria-checked', () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false } })
    expect(wrapper.find('[role="switch"]').attributes('aria-checked')).toBe('false')
  })

  it('reflects on state via aria-checked', () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: true } })
    expect(wrapper.find('[role="switch"]').attributes('aria-checked')).toBe('true')
  })

  it('emits update:modelValue with true when toggled from off', async () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false } })
    await wrapper.find('[role="switch"]').trigger('click')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
    expect(emitted![0]).toEqual([true])
  })

  it('emits update:modelValue with false when toggled from on', async () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: true } })
    await wrapper.find('[role="switch"]').trigger('click')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted![0]).toEqual([false])
  })

  it('applies disabled attribute when disabled prop is true', () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false, disabled: true } })
    const btn = wrapper.find('[role="switch"]')
    expect(btn.attributes('disabled')).toBeDefined()
  })

  it('renders label text via slot', () => {
    const wrapper = mount(ToggleSwitch, {
      props: { modelValue: false },
      slots: { default: 'Enable feature' },
    })
    expect(wrapper.text()).toContain('Enable feature')
  })
})
