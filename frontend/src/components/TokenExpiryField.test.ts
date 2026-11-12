// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount as vtuMount, type VueWrapper } from '@vue/test-utils'
import TokenExpiryField from './TokenExpiryField.vue'
import type { TokenExpiryChoice } from '../utils/apiToken'

vi.mock('../utils/format', () => ({
  formatDate: (iso: string | null) => `formatted(${iso ?? ''})`,
}))

// Local noon, so "tomorrow" is the same calendar day in every time zone.
const NOW = new Date(2026, 5, 1, 12, 0, 0)

type Wrapper = VueWrapper<InstanceType<typeof TokenExpiryField>>

/** Mounts the field with both models wired back into its props, as v-model would. */
function mount(choice: TokenExpiryChoice = 'never', date = ''): Wrapper {
  const wrapper: Wrapper = vtuMount(TokenExpiryField, {
    props: {
      idPrefix: 'tok',
      modelValue: choice,
      date,
      'onUpdate:modelValue': (v: TokenExpiryChoice) => wrapper.setProps({ modelValue: v }),
      'onUpdate:date': (v: string) => wrapper.setProps({ date: v }),
    },
  })
  return wrapper
}

describe('TokenExpiryField', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(NOW)
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('labels the select by its id prefix', () => {
    const wrapper = mount()
    expect(wrapper.get('label').attributes('for')).toBe('tok-expiry')
    expect(wrapper.get('label').text()).toBe('Expires')
    expect(wrapper.get('select').attributes('id')).toBe('tok-expiry')
  })

  it('offers every expiry choice', () => {
    const options = mount().findAll('option')
    expect(options.map((o) => [o.attributes('value'), o.text()])).toEqual([
      ['never', 'Never'],
      ['30d', '30 days'],
      ['90d', '90 days'],
      ['365d', '1 year'],
      ['date', 'On a date'],
    ])
  })

  it('says a never-expiring token works until deleted, with no date input', () => {
    const wrapper = mount('never')
    expect(wrapper.find('input[type="date"]').exists()).toBe(false)
    expect(wrapper.get('.field-hint').text()).toBe('Works until it is deleted.')
  })

  it('says when a preset length stops working', () => {
    const wrapper = mount('30d')
    const expected = new Date(NOW.getTime() + 30 * 86_400_000).toISOString()
    expect(wrapper.get('.field-hint').text()).toBe(`Stops working on formatted(${expected}).`)
    expect(wrapper.find('input[type="date"]').exists()).toBe(false)
  })

  it('emits the chosen expiry', async () => {
    const wrapper = mount('never')
    await wrapper.get('select').setValue('90d')
    expect(wrapper.emitted('update:modelValue')).toEqual([['90d']])
  })

  it('asks for a day once On a date is chosen, earliest tomorrow', async () => {
    const wrapper = mount('never')
    await wrapper.get('select').setValue('date')

    const input = wrapper.get('input[type="date"]')
    expect(input.attributes('id')).toBe('tok-expiry-date')
    expect(input.attributes('aria-label')).toBe('Expiry date')
    expect(input.attributes('min')).toBe('2026-06-02')
    expect(wrapper.get('.field-hint').text()).toBe('Pick the day it stops working.')
  })

  it('pads a single-digit month and day in the earliest date', () => {
    vi.setSystemTime(new Date(2026, 0, 3, 12, 0, 0))
    expect(mount('date').get('input[type="date"]').attributes('min')).toBe('2026-01-04')
  })

  it('rolls the earliest date over a month and year end', () => {
    vi.setSystemTime(new Date(2026, 11, 31, 12, 0, 0))
    expect(mount('date').get('input[type="date"]').attributes('min')).toBe('2027-01-01')
  })

  it('emits the picked day and says when it stops working', async () => {
    const wrapper = mount('date')
    await wrapper.get('input[type="date"]').setValue('2026-07-15')

    expect(wrapper.emitted('update:date')).toEqual([['2026-07-15']])
    const expected = new Date(2026, 6, 15, 23, 59, 59).toISOString()
    expect(wrapper.get('.field-hint').text()).toBe(`Stops working on formatted(${expected}).`)
  })
})
