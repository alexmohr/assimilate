// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import EditFormActions from './EditFormActions.vue'

describe('EditFormActions', () => {
  it('labels the save button "Save" unless told otherwise', () => {
    expect(
      mount(EditFormActions, { props: { saving: false } })
        .findAll('button')[1]
        .text(),
    ).toBe('Save')
    expect(
      mount(EditFormActions, { props: { saving: false, saveLabel: 'Save Changes' } })
        .findAll('button')[1]
        .text(),
    ).toBe('Save Changes')
  })

  it('swaps to a busy label and disables saving while in flight', () => {
    const save = mount(EditFormActions, {
      props: { saving: true, saveLabel: 'Save Changes' },
    }).findAll('button')[1]
    expect(save.text()).toBe('Saving...')
    expect(save.attributes('disabled')).toBeDefined()
  })

  it('leaves Cancel enabled while saving, so a slow request is escapable', () => {
    const cancel = mount(EditFormActions, { props: { saving: true } }).findAll('button')[0]
    expect(cancel.attributes('disabled')).toBeUndefined()
  })

  it('shows the failure only once there is one', () => {
    expect(
      mount(EditFormActions, { props: { saving: false } })
        .find('.form-error')
        .exists(),
    ).toBe(false)
    expect(
      mount(EditFormActions, { props: { saving: false, error: 'Quota too small' } })
        .find('.form-error')
        .text(),
    ).toBe('Quota too small')
  })

  it('emits cancel and save from the two buttons', async () => {
    const wrapper = mount(EditFormActions, { props: { saving: false } })
    await wrapper.findAll('button')[0].trigger('click')
    await wrapper.findAll('button')[1].trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(1)
    expect(wrapper.emitted('save')).toHaveLength(1)
  })
})
