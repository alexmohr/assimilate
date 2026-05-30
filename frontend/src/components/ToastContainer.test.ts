// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ToastContainer from './ToastContainer.vue'
import type { Toast, ToastType } from '../composables/useToast'
import { ref } from 'vue'

const mockRemove = vi.fn()
const mockToasts = ref<Toast[]>([])

vi.mock('../composables/useToast', () => ({
  useToast: () => ({
    toasts: mockToasts,
    remove: mockRemove,
  }),
}))

function makeToast(overrides: Partial<Toast> = {}): Toast {
  return {
    id: 1,
    message: 'Test message',
    type: 'info' as ToastType,
    duration: 4000,
    ...overrides,
  }
}

describe('ToastContainer', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockToasts.value = []
  })

  it('renders nothing when toasts list is empty', () => {
    const wrapper = mount(ToastContainer, { attachTo: document.body })
    expect(document.querySelectorAll('[role="alert"]').length).toBe(0)
    wrapper.unmount()
  })

  it('renders a toast item for each toast', () => {
    mockToasts.value = [makeToast({ id: 1 }), makeToast({ id: 2, message: 'Second' })]
    const wrapper = mount(ToastContainer, { attachTo: document.body })
    expect(document.querySelectorAll('[role="alert"]').length).toBe(2)
    wrapper.unmount()
  })

  it('displays the toast message text', () => {
    mockToasts.value = [makeToast({ message: 'Backup completed' })]
    const wrapper = mount(ToastContainer, { attachTo: document.body })
    expect(document.body.textContent).toContain('Backup completed')
    wrapper.unmount()
  })

  it('applies type-specific CSS class', () => {
    mockToasts.value = [makeToast({ type: 'error' })]
    const wrapper = mount(ToastContainer, { attachTo: document.body })
    const alert = document.querySelector('[role="alert"]')
    expect(alert?.classList.contains('toast-error')).toBe(true)
    wrapper.unmount()
  })

  it('calls remove with toast id when dismiss button is clicked', async () => {
    mockToasts.value = [makeToast({ id: 42 })]
    const wrapper = mount(ToastContainer, { attachTo: document.body })
    const dismissBtn = document.querySelector<HTMLButtonElement>('button[aria-label="Dismiss"]')
    expect(dismissBtn).not.toBeNull()
    dismissBtn!.click()
    await wrapper.vm.$nextTick()
    expect(mockRemove).toHaveBeenCalledWith(42)
    wrapper.unmount()
  })
})
