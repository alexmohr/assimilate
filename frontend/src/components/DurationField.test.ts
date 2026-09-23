// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import DurationField from './DurationField.vue'

function mount(modelValue: number) {
  return renderWithPlugins(DurationField, {
    props: {
      modelValue,
      units: ['minutes', 'hours', 'days'],
      inputId: 'duration',
      unitLabel: 'Duration unit',
    },
  })
}

function lastEmitted(wrapper: ReturnType<typeof mount>): number | undefined {
  const emitted = wrapper.emitted('update:modelValue') as number[][] | undefined
  return emitted?.at(-1)?.[0]
}

describe('DurationField', () => {
  it('shows a stored value in the unit it reads naturally in', () => {
    const wrapper = mount(180)
    expect((wrapper.find('#duration').element as HTMLInputElement).value).toBe('3')
    expect((wrapper.find('select').element as HTMLSelectElement).value).toBe('hours')
  })

  /**
   * Clearing the number to retype it is an ordinary edit, and must not drop the
   * unit it was shown in: "30" typed into an hours field is 30 hours.
   */
  it('keeps the unit when the number is cleared and retyped', async () => {
    const wrapper = mount(180)
    await wrapper.find('#duration').setValue('')
    await wrapper.setProps({ modelValue: lastEmitted(wrapper) })
    expect((wrapper.find('select').element as HTMLSelectElement).value).toBe('hours')

    await wrapper.find('#duration').setValue('30')
    expect(lastEmitted(wrapper)).toBe(30 * 60)
  })
})
