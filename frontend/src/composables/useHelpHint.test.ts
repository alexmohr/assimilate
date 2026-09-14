// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, h, nextTick, ref } from 'vue'
import { useHelpHint } from './useHelpHint'

/** Two independent hints in one tree, so "one at a time" has something to violate. */
const Host = defineComponent({
  setup() {
    const rootA = ref<HTMLElement | null>(null)
    const rootB = ref<HTMLElement | null>(null)
    const openA = useHelpHint(rootA)
    const openB = useHelpHint(rootB)
    return { rootA, rootB, openA, openB }
  },
  render() {
    return h('div', [
      h('div', { ref: 'rootA' }, [
        h('button', { class: 'btn-a', onClick: () => (this.openA = !this.openA) }),
        this.openA ? h('span', { class: 'pop-a' }) : null,
      ]),
      h('div', { ref: 'rootB' }, [
        h('button', { class: 'btn-b', onClick: () => (this.openB = !this.openB) }),
        this.openB ? h('span', { class: 'pop-b' }) : null,
      ]),
    ])
  },
})

describe('useHelpHint', () => {
  it('starts closed', () => {
    const wrapper = mount(Host)
    expect(wrapper.find('.pop-a').exists()).toBe(false)
    expect(wrapper.find('.pop-b').exists()).toBe(false)
  })

  it('opens on toggle', async () => {
    const wrapper = mount(Host)
    await wrapper.find('.btn-a').trigger('click')
    expect(wrapper.find('.pop-a').exists()).toBe(true)
  })

  it('closes on Escape', async () => {
    const wrapper = mount(Host)
    await wrapper.find('.btn-a').trigger('click')
    expect(wrapper.find('.pop-a').exists()).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()

    expect(wrapper.find('.pop-a').exists()).toBe(false)
  })

  it('closes on a pointerdown outside the root', async () => {
    const wrapper = mount(Host, { attachTo: document.body })
    await wrapper.find('.btn-a').trigger('click')
    expect(wrapper.find('.pop-a').exists()).toBe(true)

    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await nextTick()

    expect(wrapper.find('.pop-a').exists()).toBe(false)
    wrapper.unmount()
  })

  it('closes a hint when a different one opens, without a pointerdown between them', async () => {
    // Regression: two `HelpHint`s side by side, the second activated by
    // keyboard (Enter/Space fires click with no intervening pointerdown), so
    // useOverflowMenu's own outside-click closing does not fire for A.
    const wrapper = mount(Host)
    await wrapper.find('.btn-a').trigger('click')
    expect(wrapper.find('.pop-a').exists()).toBe(true)

    await wrapper.find('.btn-b').trigger('click')

    expect(wrapper.find('.pop-a').exists()).toBe(false)
    expect(wrapper.find('.pop-b').exists()).toBe(true)
  })

  it('leaves the other hint open when this one merely closes', async () => {
    const wrapper = mount(Host)
    await wrapper.find('.btn-a').trigger('click')
    await wrapper.find('.btn-b').trigger('click')
    expect(wrapper.find('.pop-b').exists()).toBe(true)

    // A was already closed by B opening; closing B itself must not disturb A.
    await wrapper.find('.btn-b').trigger('click')

    expect(wrapper.find('.pop-a').exists()).toBe(false)
    expect(wrapper.find('.pop-b').exists()).toBe(false)
  })
})
