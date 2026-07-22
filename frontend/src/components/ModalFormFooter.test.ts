// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ModalFormFooter from './ModalFormFooter.vue'

describe('ModalFormFooter', () => {
  it('renders submit button with given text', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, submitText: 'Create' },
    })
    expect(wrapper.text()).toContain('Create')
  })

  it('renders cancel button with default text', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, submitText: 'Save' },
    })
    expect(wrapper.text()).toContain('Cancel')
  })

  it('renders cancel button with custom text', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, submitText: 'Save', cancelText: 'Back' },
    })
    expect(wrapper.text()).toContain('Back')
  })

  it('emits cancel when cancel button is clicked', async () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, submitText: 'Save' },
    })
    await wrapper.find('button.btn-ghost').trigger('click')
    expect(wrapper.emitted('cancel')).toBeDefined()
    expect(wrapper.emitted('cancel')!.length).toBe(1)
  })

  it('shows error message when error prop is set', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: 'Something went wrong', submitting: false, submitText: 'Save' },
    })
    expect(wrapper.text()).toContain('Something went wrong')
  })

  it('disables submit button while submitting', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: true, submitText: 'Save' },
    })
    const submitBtn = wrapper.find('button[type="submit"]')
    expect(submitBtn.attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Save...')
  })

  it('disables submit button when disabled prop is true', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, disabled: true, submitText: 'Save' },
    })
    const submitBtn = wrapper.find('button[type="submit"]')
    expect(submitBtn.attributes('disabled')).toBeDefined()
  })

  it('applies btn-primary class for default variant', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, submitText: 'Save' },
    })
    const submitBtn = wrapper.find('button[type="submit"]')
    expect(submitBtn.classes()).toContain('btn-primary')
  })

  it('applies btn-danger class for danger variant', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: false, submitText: 'Delete', variant: 'danger' },
    })
    const submitBtn = wrapper.find('button[type="submit"]')
    expect(submitBtn.classes()).toContain('btn-danger')
  })

  it('shows submitting text with ellipsis', () => {
    const wrapper = mount(ModalFormFooter, {
      props: { error: null, submitting: true, submitText: 'Delete' },
    })
    expect(wrapper.text()).toContain('Delete...')
  })
})
