// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, afterEach } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, type ComponentPublicInstance } from 'vue'
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

  it('names the dialog for assistive tech via aria-labelledby', () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Named Dialog' },
      attachTo: document.body,
    })
    const dialog = document.querySelector('[role="dialog"]')!
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    const labelledBy = dialog.getAttribute('aria-labelledby')
    expect(labelledBy).toBeTruthy()
    expect(document.getElementById(labelledBy!)?.textContent?.trim()).toBe('Named Dialog')
  })

  it('gives each instance its own title id', () => {
    // A view declaring several dialogs is the case that matters: a shared id
    // would point one dialog's aria-labelledby at another dialog's heading.
    const TwoModals = defineComponent({
      components: { BaseModal },
      template: `
        <div>
          <BaseModal :open="true" title="One" />
          <BaseModal :open="true" title="Two" />
        </div>
      `,
    })
    wrapper = mount(TwoModals, { attachTo: document.body })

    const ids = Array.from(document.querySelectorAll('[role="dialog"]')).map((d) =>
      d.getAttribute('aria-labelledby'),
    )
    expect(ids).toHaveLength(2)
    expect(ids[0]).not.toBe(ids[1])
    for (const id of ids) {
      expect(document.getElementById(id!)).not.toBeNull()
    }
  })

  it('lets a custom header still name the dialog', () => {
    wrapper = mount(BaseModal, {
      props: { open: true },
      slots: {
        header: '<h2 :id="titleId" class="modal-title">Slot Title</h2>',
      },
      attachTo: document.body,
    })
    const dialog = document.querySelector('[role="dialog"]')!
    expect(dialog.getAttribute('aria-labelledby')).toBeTruthy()
  })

  it('emits close on Escape', async () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Esc Test' },
      attachTo: document.body,
    })
    await wrapper.vm.$nextTick()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('locks page scroll while open and releases it on close', async () => {
    wrapper = mount(BaseModal, {
      props: { open: false, title: 'Scroll Test' },
      attachTo: document.body,
    })
    expect(document.documentElement.style.overflow).toBe('')

    await wrapper.setProps({ open: true })
    expect(document.documentElement.style.overflow).toBe('hidden')

    await wrapper.setProps({ open: false })
    expect(document.documentElement.style.overflow).toBe('')
  })

  it('traps Tab inside the dialog', async () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Trap Test' },
      slots: { default: '<button class="first">One</button><button class="last">Two</button>' },
      attachTo: document.body,
    })
    await wrapper.vm.$nextTick()

    const focusable = Array.from(
      document.querySelectorAll<HTMLButtonElement>('.modal-dialog button'),
    )
    const last = focusable[focusable.length - 1]
    last.focus()
    expect(document.activeElement).toBe(last)

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', cancelable: true }))
    await wrapper.vm.$nextTick()
    // Tab from the last focusable wraps to the first instead of escaping.
    expect(document.activeElement).toBe(focusable[0])
  })

  it('stops listening for Escape once closed', async () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Detach Test' },
      attachTo: document.body,
    })
    await wrapper.setProps({ open: false })
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await wrapper.vm.$nextTick()
    // One close from the prop change, none from the stray key press.
    expect(wrapper.emitted('close')).toBeFalsy()
  })

  it('wraps body and footer in a form and emits submit in form mode', async () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'Form Test', form: true },
      slots: {
        default: '<input class="field-input" />',
        footer: '<button type="submit" class="go">Save</button>',
      },
      attachTo: document.body,
    })
    const form = document.querySelector<HTMLFormElement>('.modal-dialog form')
    expect(form).not.toBeNull()
    // The submit button and the field share one form, which is the point.
    expect(form!.querySelector('.field-input')).not.toBeNull()
    expect(form!.querySelector('.go')).not.toBeNull()

    form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('submit')).toBeTruthy()
  })

  it('renders no form element when form mode is off', () => {
    wrapper = mount(BaseModal, {
      props: { open: true, title: 'No Form' },
      attachTo: document.body,
    })
    expect(document.querySelector('.modal-dialog form')).toBeNull()
  })
})
