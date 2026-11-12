// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import BrowserRow from './BrowserRow.vue'

function row(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return mount(BrowserRow, { props: { title: 'etc/hosts', ...props }, slots })
}

describe('BrowserRow', () => {
  it('is a plain, unclickable div by default with a monospace title', () => {
    const wrapper = row()
    expect(wrapper.element.tagName).toBe('DIV')
    expect(wrapper.attributes('type')).toBeUndefined()
    expect(wrapper.classes()).toEqual(['browser-row'])
    expect(wrapper.get('.browser-row-title').text()).toBe('etc/hosts')
    expect(wrapper.get('.browser-row-title').classes()).toContain('browser-row-title--mono')
  })

  it('sets words in the plain font when it is not a machine name', () => {
    expect(row({ mono: false }).get('.browser-row-title').classes()).not.toContain(
      'browser-row-title--mono',
    )
  })

  it('renders as a real button that reads as clickable when it only opens something', async () => {
    const wrapper = row({ as: 'button' })
    expect(wrapper.element.tagName).toBe('BUTTON')
    expect(wrapper.attributes('type')).toBe('button')
    expect(wrapper.classes()).toContain('browser-row--clickable')

    await wrapper.trigger('click')
    const clicks = wrapper.emitted('click')
    expect(clicks).toHaveLength(1)
    expect(clicks![0]![0]).toBeInstanceOf(MouseEvent)
  })

  it('reads as clickable as a div too, so it can hold a control of its own', async () => {
    const wrapper = row({ clickable: true }, { lead: '<input type="checkbox" />' })
    expect(wrapper.element.tagName).toBe('DIV')
    expect(wrapper.classes()).toContain('browser-row--clickable')
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(true)

    await wrapper.trigger('click')
    expect(wrapper.emitted('click')).toHaveLength(1)
  })

  it('marks a selected row and dims one on its way out', () => {
    expect(row({ selected: true }).classes()).toContain('browser-row--selected')
    expect(row({ dim: true }).classes()).toContain('browser-row--dim')
    const plain = row()
    expect(plain.classes()).not.toContain('browser-row--selected')
    expect(plain.classes()).not.toContain('browser-row--dim')
  })

  it('shows the size and the time on the right, the full timestamp as a tooltip', () => {
    const wrapper = row({ size: '2.0 KB', time: '6 Nov', timeTitle: '06/11/2026, 02:00:00' })
    const side = wrapper.get('.browser-row-side')
    expect(side.get('.browser-row-size').text()).toBe('2.0 KB')
    const time = side.get('time')
    expect(time.text()).toBe('6 Nov')
    expect(time.attributes('title')).toBe('06/11/2026, 02:00:00')
  })

  it('shows only what it is given on the right, and nothing when given neither', () => {
    const sizeOnly = row({ size: '1 B' })
    expect(sizeOnly.find('.browser-row-size').exists()).toBe(true)
    expect(sizeOnly.find('time').exists()).toBe(false)

    const timeOnly = row({ time: '02:00' })
    expect(timeOnly.find('.browser-row-size').exists()).toBe(false)
    expect(timeOnly.get('time').attributes('title')).toBeUndefined()

    expect(row().find('.browser-row-side').exists()).toBe(false)
  })

  it('keeps an empty size, which is still something to show', () => {
    expect(row({ size: '' }).find('.browser-row-side').exists()).toBe(true)
  })

  it('puts the facts line under the title only when there are facts', () => {
    expect(row().find('.browser-row-facts').exists()).toBe(false)
    const wrapper = row({}, { facts: '<span>web-01</span><span>+1 KB new</span>' })
    expect(
      wrapper
        .get('.browser-row-facts')
        .findAll('span')
        .map((s) => s.text()),
    ).toEqual(['web-01', '+1 KB new'])
  })

  it('lets a slot replace the title, and places the lead before and the end after', () => {
    const wrapper = row(
      { size: '5 B' },
      {
        lead: '<i class="lead-icon" />',
        title: '<mark>etc</mark>/hosts',
        end: '<button class="end-button">Restore...</button>',
      },
    )
    expect(wrapper.get('.browser-row-title mark').text()).toBe('etc')
    expect(wrapper.get('.browser-row-title').text()).toBe('etc/hosts')

    const children = Array.from(wrapper.element.children).map((el) => el.className)
    expect(children).toEqual(['lead-icon', 'browser-row-main', 'browser-row-side', 'end-button'])
  })
})
