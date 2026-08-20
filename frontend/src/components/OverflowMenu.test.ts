// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { menuLabels, openMenu } from '../test-utils/overflowMenu'
import OverflowMenu from './OverflowMenu.vue'

/**
 * The open/close behaviour every detail header relies on. It used to be
 * asserted once per header, which is one copy per page rather than one copy
 * per behaviour.
 */
function mount() {
  return renderWithPlugins(OverflowMenu, {
    props: { label: 'More actions' },
    slots: {
      default: `<button class="overflow-menu-item" @click="run(() => {})">Logs</button>`,
    },
  })
}

describe('OverflowMenu', () => {
  it('keeps the menu closed until the toggle is pressed', async () => {
    const wrapper = mount()
    expect(menuLabels(wrapper)).toEqual([])
    expect(wrapper.find('.overflow-toggle').attributes('aria-expanded')).toBe('false')

    await openMenu(wrapper)

    expect(menuLabels(wrapper)).toEqual(['Logs'])
    expect(wrapper.find('.overflow-toggle').attributes('aria-expanded')).toBe('true')
  })

  it('names itself for assistive technology', () => {
    expect(mount().find('.overflow-toggle').attributes('aria-label')).toBe('More actions')
    expect(mount().find('.overflow-toggle').attributes('aria-haspopup')).toBe('menu')
  })

  it('closes the menu on Escape', async () => {
    const wrapper = mount()
    await openMenu(wrapper)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await flushPromises()

    expect(menuLabels(wrapper)).toEqual([])
  })

  it('closes the menu on a click outside it', async () => {
    const wrapper = mount()
    await openMenu(wrapper)

    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await flushPromises()

    expect(menuLabels(wrapper)).toEqual([])
  })

  it('leaves the menu open when the click is inside it', async () => {
    const wrapper = mount()
    await openMenu(wrapper)

    wrapper
      .find('.overflow-menu')
      .element.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await flushPromises()

    expect(menuLabels(wrapper)).toEqual(['Logs'])
  })

  it('closes the menu when an item runs its action', async () => {
    const wrapper = mount()
    await openMenu(wrapper)

    await wrapper.find('.overflow-menu-item').trigger('click')
    await flushPromises()

    expect(menuLabels(wrapper)).toEqual([])
  })
})
