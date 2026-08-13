// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ConfirmDeleteDialog from './ConfirmDeleteDialog.vue'

describe('ConfirmDeleteDialog', () => {
  it('does not render when show is false', () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: false, title: 'Delete Thing', submitting: false },
    })
    expect(wrapper.find('.overlay').exists()).toBe(false)
  })

  it('renders the title and slot content when shown', () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false },
      slots: { default: 'Are you sure you want to delete this group?' },
    })
    expect(wrapper.find('.dialog-title').text()).toBe('Delete Group')
    expect(wrapper.text()).toContain('Are you sure you want to delete this group?')
  })

  it('shows "Delete" and enables the button when not submitting', () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false },
    })
    const deleteBtn = wrapper.find('button.btn-danger')
    expect(deleteBtn.text()).toBe('Delete')
    expect(deleteBtn.attributes('disabled')).toBeUndefined()
  })

  it('shows "Deleting..." and disables the button while submitting', () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: true },
    })
    const deleteBtn = wrapper.find('button.btn-danger')
    expect(deleteBtn.text()).toBe('Deleting...')
    expect(deleteBtn.attributes('disabled')).toBeDefined()
  })

  it('emits confirm when the Delete button is clicked', async () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false },
    })
    await wrapper.find('button.btn-danger').trigger('click')
    expect(wrapper.emitted('confirm')).toBeTruthy()
  })

  it('emits cancel when the Cancel button or close button is clicked', async () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false },
    })
    await wrapper.find('button.btn-ghost').trigger('click')
    expect(wrapper.emitted('cancel')).toBeTruthy()

    await wrapper.find('button.close-btn').trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(2)
  })

  it('renders the error message when provided', () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false, error: 'Delete failed' },
    })
    expect(wrapper.find('.form-error').text()).toBe('Delete failed')
  })

  it('does not render an error element when error is not set', () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false },
    })
    expect(wrapper.find('.form-error').exists()).toBe(false)
  })

  it('emits cancel when the overlay backdrop is clicked', async () => {
    const wrapper = mount(ConfirmDeleteDialog, {
      props: { show: true, title: 'Delete Group', submitting: false },
    })
    await wrapper.find('.overlay').trigger('click')
    expect(wrapper.emitted('cancel')).toBeTruthy()
  })
})
