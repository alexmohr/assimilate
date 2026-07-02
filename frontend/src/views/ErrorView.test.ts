// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ErrorView from './ErrorView.vue'
import { storeErrorDetails } from '../utils/errorDetails'

let routeQuery: Record<string, string> = {}

vi.mock('vue-router', async (importOriginal) => {
  const actual: Record<string, unknown> = await importOriginal()
  return {
    ...actual,
    useRouter: vi.fn(() => ({ push: vi.fn() })),
    useRoute: vi.fn(() => ({ query: routeQuery })),
  }
})

describe('ErrorView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    sessionStorage.clear()
    routeQuery = {}
  })

  it('shows the default status code and message when no query is set', () => {
    const wrapper = mount(ErrorView)
    expect(wrapper.find('.error-code').text()).toBe('500')
    expect(wrapper.find('.error-message').text()).toContain('Something went wrong')
  })

  it('shows the status code and message from the route query', () => {
    routeQuery = { code: '403', message: 'Forbidden' }
    const wrapper = mount(ErrorView)
    expect(wrapper.find('.error-code').text()).toBe('403')
    expect(wrapper.find('.error-message').text()).toBe('Forbidden')
  })

  it('does not render a details toggle when no error details were stashed', () => {
    const wrapper = mount(ErrorView)
    expect(wrapper.find('.error-details').exists()).toBe(false)
  })

  it('renders a collapsible details section with a frontend source label', async () => {
    storeErrorDetails({
      source: 'frontend',
      name: 'TypeError',
      message: "Cannot read properties of undefined (reading 'foo')",
      stack: "TypeError: Cannot read properties of undefined (reading 'foo')\n    at App.vue:10",
    })

    const wrapper = mount(ErrorView)
    expect(wrapper.find('.error-source').text()).toBe('Frontend error')

    const toggle = wrapper.find('.error-details .error-toggle')
    expect(toggle.exists()).toBe(true)
    expect(wrapper.find('.error-details .error-pre').exists()).toBe(false)

    await toggle.trigger('click')

    const pre = wrapper.find('.error-details .error-pre')
    expect(pre.exists()).toBe(true)
    expect(pre.text()).toContain('TypeError')
    expect(pre.text()).toContain('at App.vue:10')
  })

  it('renders a backend source label when the stashed error came from the backend', () => {
    storeErrorDetails({ source: 'backend', message: 'Internal server error' })

    const wrapper = mount(ErrorView)
    expect(wrapper.find('.error-source').text()).toBe('Backend error')
  })
})
