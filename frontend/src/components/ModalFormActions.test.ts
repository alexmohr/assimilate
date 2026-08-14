// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ModalFormActions from './ModalFormActions.vue'

describe('ModalFormActions', () => {
  it('renders the submit label when not submitting', () => {
    const wrapper = mount(ModalFormActions, {
      props: { submitting: false, submitLabel: 'Create', submittingLabel: 'Creating...' },
    })
    expect(wrapper.text()).toContain('Create')
    expect(wrapper.text()).not.toContain('Creating...')
  })

  it('renders the submitting label while submitting', () => {
    const wrapper = mount(ModalFormActions, {
      props: { submitting: true, submitLabel: 'Create', submittingLabel: 'Creating...' },
    })
    expect(wrapper.text()).toContain('Creating...')
  })

  it('disables the submit button while submitting', () => {
    const wrapper = mount(ModalFormActions, {
      props: { submitting: true, submitLabel: 'Save', submittingLabel: 'Saving...' },
    })
    const submit = wrapper.find('button[type="submit"]')
    expect(submit.attributes('disabled')).toBeDefined()
  })

  it('disables the submit button when disabled prop is set', () => {
    const wrapper = mount(ModalFormActions, {
      props: {
        submitting: false,
        disabled: true,
        submitLabel: 'Save',
        submittingLabel: 'Saving...',
      },
    })
    const submit = wrapper.find('button[type="submit"]')
    expect(submit.attributes('disabled')).toBeDefined()
  })

  it('emits cancel when the Cancel button is clicked', async () => {
    const wrapper = mount(ModalFormActions, {
      props: { submitting: false, submitLabel: 'Save', submittingLabel: 'Saving...' },
    })
    await wrapper.find('button.btn-ghost').trigger('click')
    expect(wrapper.emitted('cancel')).toBeTruthy()
  })

  it('renders the submit button as type="button" and emits confirm on click when type is button', async () => {
    const wrapper = mount(ModalFormActions, {
      props: {
        submitting: false,
        type: 'button',
        submitLabel: 'Delete',
        submittingLabel: 'Deleting...',
      },
    })
    const submit = wrapper.find('button[type="button"].btn-primary')
    expect(submit.exists()).toBe(true)
    await submit.trigger('click')
    expect(wrapper.emitted('confirm')).toBeTruthy()
  })

  it('renders the error message when provided', () => {
    const wrapper = mount(ModalFormActions, {
      props: {
        submitting: false,
        error: 'Something went wrong',
        submitLabel: 'Save',
        submittingLabel: 'Saving...',
      },
    })
    expect(wrapper.find('.form-error').text()).toBe('Something went wrong')
  })

  it('does not render an error element when error is not set', () => {
    const wrapper = mount(ModalFormActions, {
      props: { submitting: false, submitLabel: 'Save', submittingLabel: 'Saving...' },
    })
    expect(wrapper.find('.form-error').exists()).toBe(false)
  })
})
