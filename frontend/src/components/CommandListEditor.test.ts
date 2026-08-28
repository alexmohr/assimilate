// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import CommandListEditor from './CommandListEditor.vue'

function mount(modelValue: string[] = []) {
  return renderWithPlugins(CommandListEditor, {
    props: {
      modelValue,
      'onUpdate:modelValue': () => {},
    },
  })
}

describe('CommandListEditor', () => {
  it('renders one field per command', () => {
    const wrapper = mount(['echo one', 'echo two'])
    expect(wrapper.findAll('textarea')).toHaveLength(2)
  })

  it('renders no fields for an empty list', () => {
    const wrapper = mount([])
    expect(wrapper.findAll('textarea')).toHaveLength(0)
  })

  it('preserves newlines within a single command, unlike a shared textarea', () => {
    const script = 'umount -l /mnt/pve/x\npvesm status --storage x || exit 1'
    const wrapper = mount([script])
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe(script)
  })

  it('adds a new empty command when "+ Add command" is clicked', async () => {
    const wrapper = mount(['first'])
    await wrapper.find('.btn-ghost').trigger('click')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['first', '']])
  })

  it('removes a command without touching the others', async () => {
    const wrapper = mount(['first', 'second', 'third'])
    await wrapper.findAll('.btn-danger')[1].trigger('click')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['first', 'third']])
  })

  it('writes an edited command back at its own index', async () => {
    const wrapper = mount(['first', 'second'])
    await wrapper.findAll('textarea')[1].setValue('edited')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['first', 'edited']])
  })
})
