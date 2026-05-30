// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, afterEach } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import type { ComponentPublicInstance } from 'vue'
import BaseModal from './BaseModal.vue'

let wrapper: VueWrapper<ComponentPublicInstance> | null = null

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('BaseModal', () => {
  it('renders nothing when open is false', () => {
    wrapper = mount(BaseModal, {
      props: { open: false },
      attachTo: document.body,
    })
    expect(document.querySelector('[role="dialog"]')).toBeNull()
  })

  it('renders dialog when open is true', () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Test Modal' },
      attachTo: document.body,
    })
    expect(document.querySelector('[role="dialog"]')).not.toBeNull()
  })

  it('renders title text', () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'My Title' },
      attachTo: document.body,
    })
    expect(document.body.textContent).toContain('My Title')
  })

  it('renders default slot content', () => {
    wrapper = mount(BaseModal, {
      props: { open: true },
      slots: { default: '<p class="slot-content">Body text</p>' },
      attachTo: document.body,
    })
    expect(document.querySelector('.slot-content')).not.toBeNull()
  })

  it('emits close when close button is clicked', async () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Close Test' },
      attachTo: document.body,
    })
    const closeBtn = document.querySelector<HTMLButtonElement>('button[aria-label="Close"]')
    expect(closeBtn).not.toBeNull()
    closeBtn!.click()
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('emits close when backdrop mousedown.self fires', async () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Backdrop Test' },
      attachTo: document.body,
    })
    const backdrop = document.querySelector<HTMLElement>('.modal-backdrop')
    expect(backdrop).not.toBeNull()
    const event = new MouseEvent('mousedown', { bubbles: true })
    Object.defineProperty(event, 'target', { value: backdrop })
    backdrop!.dispatchEvent(event)
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('applies size class to dialog', () => {
    wrapper = mount(BaseModal, {
      props: { open: true, size: 'lg' },
      attachTo: document.body,
    })
    const dialog = document.querySelector('[role="dialog"]')
    expect(dialog?.classList.contains('modal-lg')).toBe(true)
  })
})
