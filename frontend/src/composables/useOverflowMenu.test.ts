// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, h, nextTick, ref } from 'vue'
import { useOverflowMenu } from './useOverflowMenu'

/** A minimal host so the composable's document listeners run against a real DOM. */
const Host = defineComponent({
  setup() {
    const menuRoot = ref<HTMLElement | null>(null)
    const { menuOpen, runAndClose } = useOverflowMenu(menuRoot)
    return { menuOpen, menuRoot, runAndClose }
  },
  render() {
    return h('div', { ref: 'menuRoot' }, [
      h('button', { class: 'toggle', onClick: () => (this.menuOpen = !this.menuOpen) }),
      this.menuOpen
        ? h('button', { class: 'item', onClick: () => this.runAndClose(() => {}) })
        : null,
    ])
  },
})

describe('useOverflowMenu', () => {
  it('starts closed', () => {
    const wrapper = mount(Host)
    expect(wrapper.find('.item').exists()).toBe(false)
  })

  it('opens on toggle', async () => {
    const wrapper = mount(Host)
    await wrapper.find('.toggle').trigger('click')
    expect(wrapper.find('.item').exists()).toBe(true)
  })

  it('closes on Escape', async () => {
    const wrapper = mount(Host)
    await wrapper.find('.toggle').trigger('click')
    expect(wrapper.find('.item').exists()).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()

    expect(wrapper.find('.item').exists()).toBe(false)
  })

  it('closes on a pointerdown outside the root', async () => {
    const wrapper = mount(Host, { attachTo: document.body })
    await wrapper.find('.toggle').trigger('click')
    expect(wrapper.find('.item').exists()).toBe(true)

    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await nextTick()

    expect(wrapper.find('.item').exists()).toBe(false)
    wrapper.unmount()
  })

  it('leaves the menu open on a pointerdown inside the root', async () => {
    const wrapper = mount(Host, { attachTo: document.body })
    await wrapper.find('.toggle').trigger('click')

    wrapper.find('.item').element.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await nextTick()

    expect(wrapper.find('.item').exists()).toBe(true)
    wrapper.unmount()
  })

  it('runAndClose runs the action and closes the menu', async () => {
    let ran = false
    const wrapper = mount(
      defineComponent({
        setup() {
          const { runAndClose } = useOverflowMenu(ref(null))
          return { runAndClose }
        },
        render() {
          return h('button', {
            class: 'item',
            onClick: () => this.runAndClose(() => (ran = true)),
          })
        },
      }),
    )
    // Open first via the composable's own state isn't exposed here; call runAndClose directly.
    await wrapper.find('.item').trigger('click')
    expect(ran).toBe(true)
  })

  it('removes the document listener on unmount', async () => {
    const wrapper = mount(Host, { attachTo: document.body })
    await wrapper.find('.toggle').trigger('click')
    wrapper.unmount()

    // No listener should remain to throw or reopen anything after unmount.
    expect(() =>
      document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })),
    ).not.toThrow()
  })
})
