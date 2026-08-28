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

  // The old single shared textarea always showed its placeholder as
  // guidance; a still-empty list of rows here must not go silent just
  // because there is nothing yet to click "+ Add command" and see it on.
  it('shows the placeholder text as a hint when the list is empty', () => {
    const wrapper = renderWithPlugins(CommandListEditor, {
      props: {
        modelValue: [],
        placeholder: 'e.g. systemctl stop myapp',
        'onUpdate:modelValue': () => {},
      },
    })
    expect(wrapper.find('.field-hint').text()).toBe('e.g. systemctl stop myapp')
  })

  it('hides the empty-list hint once a row exists', () => {
    const wrapper = mount(['first'])
    expect(wrapper.find('.field-hint').exists()).toBe(false)
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

  // A partial external reassignment (only one entry actually differs) must
  // not churn the rows that didn't change: minting a fresh id for every row,
  // not just the changed one, would destroy and recreate the untouched rows'
  // DOM nodes too - the same focus/identity loss the stable per-row id keying
  // exists to prevent.
  it('keeps the DOM node for rows whose value is unchanged when only one entry differs', async () => {
    const wrapper = mount(['a', 'b', 'c'])
    const before = wrapper.findAll('textarea')
    const firstEl = before[0].element
    const thirdEl = before[2].element

    await wrapper.setProps({ modelValue: ['a', 'X', 'c'] })

    const after = wrapper.findAll('textarea')
    expect(after[0].element).toBe(firstEl)
    expect((after[1].element as HTMLTextAreaElement).value).toBe('X')
    expect(after[2].element).toBe(thirdEl)
  })

  // Each row's own name is numbered - not just the given ariaLabel repeated
  // verbatim on every row - so a screen reader user can tell "Pre-backup
  // commands 1" apart from "Pre-backup commands 2" once there's more than one.
  it('applies the given accessible name to every row, numbered by position', () => {
    const wrapper = renderWithPlugins(CommandListEditor, {
      props: {
        modelValue: ['first', 'second'],
        ariaLabel: 'Pre-backup commands',
        'onUpdate:modelValue': () => {},
      },
    })
    const textareas = wrapper.findAll('textarea')
    expect(textareas[0].attributes('aria-label')).toBe('Pre-backup commands 1')
    expect(textareas[1].attributes('aria-label')).toBe('Pre-backup commands 2')
  })

  // Same reasoning for the remove buttons: identical labels make voice
  // control ("click Remove command") and screen-reader navigation ambiguous
  // once there's more than one row.
  it('numbers each row remove button so they are distinguishable', () => {
    const wrapper = mount(['first', 'second'])
    const buttons = wrapper.findAll('.btn-danger')
    expect(buttons[0].attributes('aria-label')).toBe('Remove command 1')
    expect(buttons[1].attributes('aria-label')).toBe('Remove command 2')
  })
})
