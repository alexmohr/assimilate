// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import MergeClientDialog from './MergeClientDialog.vue'

vi.mock('./BaseModal.vue', () => ({
  default: {
    name: 'BaseModal',
    props: ['open', 'title', 'size'],
    emits: ['close'],
    template: `
      <div v-if="open" data-testid="modal">
        <slot />
        <slot name="footer" />
      </div>
    `,
  },
}))

interface ClientOption {
  id: number
  hostname: string
  display_name: string | null
}

const CLIENTS: ClientOption[] = [
  { id: 1, hostname: 'web-server-01', display_name: 'Web Server' },
  { id: 2, hostname: 'db-server-01', display_name: null },
  { id: 3, hostname: 'media-store-01', display_name: 'Media Store' },
]

function mountDialog(open: boolean = true): ReturnType<typeof mount> {
  return mount(MergeClientDialog, {
    props: { open, clients: CLIENTS },
    attachTo: document.body,
  })
}

describe('MergeClientDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders nothing when open is false', () => {
    const wrapper = mountDialog(false)
    expect(wrapper.find('[data-testid="modal"]').exists()).toBe(false)
  })

  it('renders dialog when open is true', () => {
    const wrapper = mountDialog()
    expect(wrapper.find('[data-testid="modal"]').exists()).toBe(true)
  })

  it('renders source and target select elements', () => {
    const wrapper = mountDialog()
    expect(wrapper.find('#merge-source').exists()).toBe(true)
    expect(wrapper.find('#merge-target').exists()).toBe(true)
  })

  it('lists all clients in source select', () => {
    const wrapper = mountDialog()
    const options = wrapper.find('#merge-source').findAll('option')
    expect(options.length).toBe(CLIENTS.length + 1)
  })

  it('renders Submit (Merge) button', () => {
    const wrapper = mountDialog()
    const buttons = wrapper.findAll('button')
    const mergeBtn = buttons.find((b) => b.text() === 'Merge')
    expect(mergeBtn).toBeTruthy()
  })

  it('Merge button is disabled when no source/target selected', () => {
    const wrapper = mountDialog()
    const mergeBtn = wrapper.findAll('button').find((b) => b.text() === 'Merge')
    expect(mergeBtn?.attributes('disabled')).toBeDefined()
  })

  it('emits merge event with sourceId and targetId when submitted', async () => {
    const wrapper = mountDialog()
    const sourceSelect = wrapper.find('#merge-source')
    const targetSelect = wrapper.find('#merge-target')

    await sourceSelect.setValue('1')
    await targetSelect.setValue('2')

    const mergeBtn = wrapper.findAll('button').find((b) => b.text() === 'Merge')
    await mergeBtn?.trigger('click')

    const emitted = wrapper.emitted('merge')
    expect(emitted).toBeTruthy()
    expect((emitted as Array<[{ sourceId: number; targetId: number }]>)[0][0]).toEqual({
      sourceId: 1,
      targetId: 2,
    })
  })

  it('emits close when Cancel is clicked', async () => {
    const wrapper = mountDialog()
    const cancelBtn = wrapper.findAll('button').find((b) => b.text() === 'Cancel')
    await cancelBtn?.trigger('click')
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('displays client hostname when display_name is null', () => {
    const wrapper = mountDialog()
    const options = wrapper.find('#merge-source').findAll('option')
    const texts = options.map((o) => o.text())
    expect(texts.some((t) => t.includes('db-server-01'))).toBe(true)
  })

  it('displays display_name with hostname when display_name is set', () => {
    const wrapper = mountDialog()
    const options = wrapper.find('#merge-source').findAll('option')
    const texts = options.map((o) => o.text())
    expect(texts.some((t) => t.includes('Web Server') && t.includes('web-server-01'))).toBe(true)
  })
})
