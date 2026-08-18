// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ErrorPage from './ErrorPage.vue'

const push = vi.fn()

vi.mock('vue-router', async (importOriginal) => {
  const actual: Record<string, unknown> = await importOriginal()
  return { ...actual, useRouter: vi.fn(() => ({ push })) }
})

const BASE = { code: '500', title: 'Error', message: 'Something broke.' }

describe('ErrorPage', () => {
  beforeEach(() => {
    push.mockClear()
  })

  it('renders the code, title and message', () => {
    const wrapper = mount(ErrorPage, { props: BASE })
    expect(wrapper.find('.error-code').text()).toBe('500')
    expect(wrapper.find('.error-title').text()).toBe('Error')
    expect(wrapper.find('.error-message').text()).toBe('Something broke.')
  })

  it('tones the numeral danger by default and accent on request', () => {
    expect(mount(ErrorPage, { props: BASE }).find('.error-code').classes()).toContain(
      'error-code--danger',
    )
    expect(
      mount(ErrorPage, { props: { ...BASE, tone: 'accent' as const } })
        .find('.error-code')
        .classes(),
    ).toContain('error-code--accent')
  })

  it('renders the source slot between the title and the message', () => {
    const wrapper = mount(ErrorPage, {
      props: BASE,
      slots: { source: '<p class="error-source">Backend error</p>' },
    })
    expect(wrapper.find('.error-source').text()).toBe('Backend error')
  })

  it('omits the source block when the slot is unused', () => {
    expect(mount(ErrorPage, { props: BASE }).find('.error-source').exists()).toBe(false)
  })

  it('renders default-slot detail below the message', () => {
    const wrapper = mount(ErrorPage, {
      props: BASE,
      slots: { default: '<div class="detail">stack trace</div>' },
    })
    expect(wrapper.find('.detail').text()).toBe('stack trace')
  })

  it('navigates home from the action button', async () => {
    const wrapper = mount(ErrorPage, { props: BASE })
    await wrapper.find('button').trigger('click')
    expect(push).toHaveBeenCalledWith('/')
  })
})
