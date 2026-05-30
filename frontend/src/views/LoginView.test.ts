// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import LoginView from './LoginView.vue'
import { useAuthStore } from '../stores/auth'

vi.mock('vue-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('vue-router')>()
  return {
    ...actual,
    useRouter: vi.fn(() => ({ push: vi.fn() })),
    useRoute: vi.fn(() => ({ query: {} })),
  }
})

describe('LoginView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders username and password inputs and submit button', () => {
    const wrapper = renderWithPlugins(LoginView)
    expect(wrapper.find('#username').exists()).toBe(true)
    expect(wrapper.find('#password').exists()).toBe(true)
    expect(wrapper.find('button[type="submit"]').exists()).toBe(true)
  })

  it('allows typing into username and password fields', async () => {
    const wrapper = renderWithPlugins(LoginView)
    const usernameInput = wrapper.find<HTMLInputElement>('#username')
    const passwordInput = wrapper.find<HTMLInputElement>('#password')

    await usernameInput.setValue('testuser')
    await passwordInput.setValue('secret123')

    expect(usernameInput.element.value).toBe('testuser')
    expect(passwordInput.element.value).toBe('secret123')
  })

  it('calls authStore.login on form submit', async () => {
    const wrapper = renderWithPlugins(LoginView)
    const authStore = useAuthStore()

    await wrapper.find('#username').setValue('admin')
    await wrapper.find('#password').setValue('password123')
    await wrapper.find('form').trigger('submit.prevent')

    expect(authStore.login).toHaveBeenCalledWith('admin', 'password123')
  })

  it('shows an error message when login fails', async () => {
    const wrapper = renderWithPlugins(LoginView)
    const authStore = useAuthStore()
    vi.mocked(authStore.login).mockRejectedValueOnce(new Error('Invalid credentials'))

    await wrapper.find('#username').setValue('admin')
    await wrapper.find('#password').setValue('wrongpass')
    await wrapper.find('form').trigger('submit.prevent')

    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    const errorEl = wrapper.find('.login-error')
    expect(errorEl.exists()).toBe(true)
    expect(errorEl.text()).toContain('Invalid credentials')
  })
})
