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

  // Regression test for keying rows by array index: removing a middle row
  // would make Vue reuse the DOM node at the highest index (destroying
  // whichever field had focus) and force-patch every other node's value in
  // place instead, corrupting the browser's native undo history for fields
  // the user never touched.
  it('keeps the DOM node for every untouched row when a middle row is removed', async () => {
    const wrapper = mount(['first', 'second', 'third'])
    const before = wrapper.findAll('textarea')
    const firstEl = before[0].element
    const thirdEl = before[2].element

    await wrapper.findAll('.btn-danger')[1].trigger('click')

    const after = wrapper.findAll('textarea')
    expect(after).toHaveLength(2)
    expect(after[0].element).toBe(firstEl)
    expect(after[1].element).toBe(thirdEl)
  })

  // Covers the other half of the parent/child sync: an external reassignment
  // of modelValue (e.g. loading a different schedule into an already-mounted
  // editor) must regenerate rows to match, not just the initial mount.
  it('regenerates rows when the parent replaces modelValue with different content', async () => {
    const wrapper = mount(['first'])
    await wrapper.setProps({ modelValue: ['second', 'third'] })

    const textareas = wrapper.findAll('textarea')
    expect(textareas).toHaveLength(2)
    expect((textareas[0].element as HTMLTextAreaElement).value).toBe('second')
    expect((textareas[1].element as HTMLTextAreaElement).value).toBe('third')
  })

  // The sameContent guard exists specifically to make this a no-op: without
  // it, the editor's own emit echoing back through v-model would regenerate
  // every row's id and DOM node on every keystroke.
  it('does not regenerate rows when modelValue is reassigned with the same content', async () => {
    const wrapper = mount(['first'])
    const beforeEl = wrapper.find('textarea').element

    await wrapper.setProps({ modelValue: ['first'] })

    expect(wrapper.find('textarea').element).toBe(beforeEl)
  })

  it('applies the given accessible name to every row', () => {
    const wrapper = renderWithPlugins(CommandListEditor, {
      props: {
        modelValue: ['first', 'second'],
        ariaLabel: 'Pre-backup commands',
        'onUpdate:modelValue': () => {},
      },
    })
    for (const textarea of wrapper.findAll('textarea')) {
      expect(textarea.attributes('aria-label')).toBe('Pre-backup commands')
    }
  })
})
