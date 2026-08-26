// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import FilterSyntaxHelp from './FilterSyntaxHelp.vue'
import { FILTER_FIELD_HELP } from '../utils/filterQuery'

// BaseModal teleports to body, so assertions read from the document rather
// than from the wrapper, and each mount has to clean up after itself.
const mounted: ReturnType<typeof mount>[] = []

function open(): void {
  const wrapper = mount(FilterSyntaxHelp, { attachTo: document.body })
  mounted.push(wrapper)
  wrapper.find('button[aria-label="Filter syntax"]').trigger('click')
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
})

describe('FilterSyntaxHelp', () => {
  it('keeps the help hidden until the button is clicked', () => {
    const wrapper = mount(FilterSyntaxHelp, { attachTo: document.body })
    mounted.push(wrapper)
    expect(document.body.textContent).not.toContain('Filter syntax')
  })

  it('documents every field the parser understands', async () => {
    open()
    await new Promise((resolve) => setTimeout(resolve, 0))

    const text = document.body.textContent ?? ''
    expect(text).toContain('Filter syntax')
    for (const row of FILTER_FIELD_HELP) {
      expect(text).toContain(row.example)
      expect(text).toContain(row.description)
    }
  })

  it('explains that a pipe is an OR', async () => {
    open()
    await new Promise((resolve) => setTimeout(resolve, 0))

    const text = document.body.textContent ?? ''
    expect(text).toContain('agent:k3s | agent:nas')
    expect(text).toContain('either may match')
  })

  it('closes on the modal close button, leaving the button clickable again', async () => {
    open()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.body.textContent).toContain('Filter syntax')

    const closeButton = document.querySelector<HTMLButtonElement>('.modal-close')
    closeButton?.click()
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(document.body.textContent).not.toContain('Filter syntax')

    // Re-opens cleanly rather than being stuck closed.
    document.querySelector<HTMLButtonElement>('button[aria-label="Filter syntax"]')?.click()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.body.textContent).toContain('Filter syntax')
  })
})
