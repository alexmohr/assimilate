// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import CronBuilder from './CronBuilder.vue'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: () => 'UTC',
}))

vi.mock('../utils/cron', () => ({
  cronToHuman: (expr: string): string => {
    if (expr === '0 2 * * *') return 'Daily at 02:00'
    if (expr === '0 */6 * * *') return 'Every 6 hours'
    return ''
  },
}))

function mountCronBuilder(modelValue: string): ReturnType<typeof mount> {
  return mount(CronBuilder, {
    props: { modelValue },
  })
}

describe('CronBuilder', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the cron expression input with the given modelValue', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    const input = wrapper.find('input.cron-input')
    expect(input.exists()).toBe(true)
    expect(input.attributes('value')).toBe('0 2 * * *')
  })

  it('renders the hint text', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.text()).toContain('minute hour day-of-month month day-of-week')
  })

  it('shows Helper toggle button', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    const btn = wrapper.find('button.helper-toggle')
    expect(btn.exists()).toBe(true)
    expect(btn.text()).toBe('Helper')
  })

  it('emits update:modelValue when user types in the input', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    const input = wrapper.find('input.cron-input')
    const el = input.element as HTMLInputElement
    el.value = '0 3 * * *'
    await input.trigger('input')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
    expect((emitted as string[][])[0][0]).toBe('0 3 * * *')
  })

  it('shows validation error for invalid cron expression', () => {
    const wrapper = mountCronBuilder('invalid')
    expect(wrapper.find('.cron-error').exists()).toBe(true)
  })

  it('does not show error for valid 5-field cron', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.find('.cron-error').exists()).toBe(false)
  })

  it('shows human description preview for valid cron', () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.text()).toContain('Daily at 02:00')
  })

  it('toggles the helper panel when Helper button is clicked', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    expect(wrapper.find('.helper-panel').exists()).toBe(false)
    await wrapper.find('button.helper-toggle').trigger('click')
    expect(wrapper.find('.helper-panel').exists()).toBe(true)
    expect(wrapper.find('button.helper-toggle').text()).toBe('Hide Helper')
  })

  it('emits the helper expression when Apply is clicked', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    await wrapper.find('button.helper-toggle').trigger('click')
    await wrapper.find('button.helper-apply-btn').trigger('click')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
  })

  it('parses daily cron into helper fields correctly', async () => {
    const wrapper = mountCronBuilder('0 2 * * *')
    await wrapper.find('button.helper-toggle').trigger('click')
    const select = wrapper.find('select.helper-select')
    expect(select.exists()).toBe(true)
    expect((select.element as HTMLSelectElement).value).toBe('daily')
  })

  it('parses hourly cron into helper fields correctly', async () => {
    const wrapper = mountCronBuilder('0 */6 * * *')
    await wrapper.find('button.helper-toggle').trigger('click')
    const select = wrapper.find('select.helper-select')
    expect((select.element as HTMLSelectElement).value).toBe('hourly')
  })
})
