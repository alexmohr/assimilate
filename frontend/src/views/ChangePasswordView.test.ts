// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ChangePasswordView from './ChangePasswordView.vue'
import { useAuthStore } from '../stores/auth'

vi.mock('vue-router', async (importOriginal) => {
  const actual: Record<string, unknown> = await importOriginal()
  return {
    ...actual,
    useRouter: vi.fn(() => ({ push: vi.fn() })),
    useRoute: vi.fn(() => ({ query: {} })),
  }
})

describe('ChangePasswordView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders new password and confirm password fields and submit button', () => {
    const wrapper = renderWithPlugins(ChangePasswordView)
    expect(wrapper.find('#new-password').exists()).toBe(true)
    expect(wrapper.find('#confirm-password').exists()).toBe(true)
    expect(wrapper.find('button[type="submit"]').exists()).toBe(true)
  })

  it('shows an error when passwords do not match', async () => {
    const wrapper = renderWithPlugins(ChangePasswordView)

    await wrapper.find('#new-password').setValue('password123')
    await wrapper.find('#confirm-password').setValue('different456')
    await wrapper.find('form').trigger('submit.prevent')

    await wrapper.vm.$nextTick()

    const errorEl = wrapper.find('.login-error')
    expect(errorEl.exists()).toBe(true)
    expect(errorEl.text()).toContain('do not match')
  })

  it('shows an error when password is too short', async () => {
    const wrapper = renderWithPlugins(ChangePasswordView)

    await wrapper.find('#new-password').setValue('short')
    await wrapper.find('#confirm-password').setValue('short')
    await wrapper.find('form').trigger('submit.prevent')

    await wrapper.vm.$nextTick()

    const errorEl = wrapper.find('.login-error')
    expect(errorEl.exists()).toBe(true)
    expect(errorEl.text()).toContain('at least 8 characters')
  })

  it('calls authStore.changePassword with valid matching passwords', async () => {
    const wrapper = renderWithPlugins(ChangePasswordView)
    const authStore = useAuthStore()

    await wrapper.find('#new-password').setValue('newpassword1')
    await wrapper.find('#confirm-password').setValue('newpassword1')
    await wrapper.find('form').trigger('submit.prevent')

    await wrapper.vm.$nextTick()

    expect(authStore.changePassword).toHaveBeenCalledWith('newpassword1')
  })

  it('shows an error message when the API call fails', async () => {
    const wrapper = renderWithPlugins(ChangePasswordView)
    const authStore = useAuthStore()
    vi.mocked(authStore.changePassword).mockRejectedValueOnce(new Error('Server error'))

    await wrapper.find('#new-password').setValue('newpassword1')
    await wrapper.find('#confirm-password').setValue('newpassword1')
    await wrapper.find('form').trigger('submit.prevent')

    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    const errorEl = wrapper.find('.login-error')
    expect(errorEl.exists()).toBe(true)
    expect(errorEl.text()).toContain('Server error')
  })
})
