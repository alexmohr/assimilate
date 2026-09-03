// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import { hookCommand, MAX_HOOK_COMMAND_TIMEOUT_SECONDS } from '../utils/hookCommands'
import CommandListEditor from './CommandListEditor.vue'
import type { HookCommand } from '../types/generated'

function mount(modelValue: HookCommand[] = []) {
  return renderWithPlugins(CommandListEditor, {
    props: {
      modelValue,
      'onUpdate:modelValue': () => {},
    },
  })
}

describe('CommandListEditor', () => {
  it('renders one field per command', () => {
    const wrapper = mount([hookCommand('echo one'), hookCommand('echo two')])
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
    expect(wrapper.find('p.field-hint').text()).toBe('e.g. systemctl stop myapp')
  })

  // Scoped to the paragraph: every row carries its own `.field-hint` spans
  // labelling the timeout field, so the bare class no longer identifies the
  // empty-list hint on its own.
  it('hides the empty-list hint once a row exists', () => {
    const wrapper = mount([hookCommand('first')])
    expect(wrapper.find('p.field-hint').exists()).toBe(false)
  })

  it('preserves newlines within a single command, unlike a shared textarea', () => {
    const script = 'umount -l /mnt/pve/x\npvesm status --storage x || exit 1'
    const wrapper = mount([hookCommand(script)])
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe(script)
  })

  it('adds a new empty command when "+ Add command" is clicked', async () => {
    const wrapper = mount([hookCommand('first')])
    await wrapper.find('.btn-ghost').trigger('click')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([
      [hookCommand('first'), hookCommand('')],
    ])
  })

  it('removes a command without touching the others', async () => {
    const wrapper = mount([hookCommand('first'), hookCommand('second'), hookCommand('third')])
    await wrapper.findAll('.btn-danger')[1].trigger('click')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([
      [hookCommand('first'), hookCommand('third')],
    ])
  })

  it('writes an edited command back at its own index', async () => {
    const wrapper = mount([hookCommand('first'), hookCommand('second')])
    await wrapper.findAll('textarea')[1].setValue('edited')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([
      [hookCommand('first'), hookCommand('edited')],
    ])
  })

  // Regression test for keying rows by array index: removing a middle row
  // would make Vue reuse the DOM node at the highest index (destroying
  // whichever field had focus) and force-patch every other node's value in
  // place instead, corrupting the browser's native undo history for fields
  // the user never touched.
  it('keeps the DOM node for every untouched row when a middle row is removed', async () => {
    const wrapper = mount([hookCommand('first'), hookCommand('second'), hookCommand('third')])
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
    const wrapper = mount([hookCommand('first')])
    await wrapper.setProps({ modelValue: [hookCommand('second'), hookCommand('third')] })

    const textareas = wrapper.findAll('textarea')
    expect(textareas).toHaveLength(2)
    expect((textareas[0].element as HTMLTextAreaElement).value).toBe('second')
    expect((textareas[1].element as HTMLTextAreaElement).value).toBe('third')
  })

  // The sameContent guard exists specifically to make this a no-op: without
  // it, the editor's own emit echoing back through v-model would regenerate
  // every row's id and DOM node on every keystroke.
  it('does not regenerate rows when modelValue is reassigned with the same content', async () => {
    const wrapper = mount([hookCommand('first')])
    const beforeEl = wrapper.find('textarea').element

    await wrapper.setProps({ modelValue: [hookCommand('first')] })

    expect(wrapper.find('textarea').element).toBe(beforeEl)
  })

  // A partial external reassignment (only one entry actually differs) must
  // not churn the rows that didn't change: minting a fresh id for every row,
  // not just the changed one, would destroy and recreate the untouched rows'
  // DOM nodes too - the same focus/identity loss the stable per-row id keying
  // exists to prevent.
  it('keeps the DOM node for rows whose value is unchanged when only one entry differs', async () => {
    const wrapper = mount([hookCommand('a'), hookCommand('b'), hookCommand('c')])
    const before = wrapper.findAll('textarea')
    const firstEl = before[0].element
    const thirdEl = before[2].element

    await wrapper.setProps({ modelValue: [hookCommand('a'), hookCommand('X'), hookCommand('c')] })

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
        modelValue: [hookCommand('first'), hookCommand('second')],
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
    const wrapper = mount([hookCommand('first'), hookCommand('second')])
    const buttons = wrapper.findAll('.btn-danger')
    expect(buttons[0].attributes('aria-label')).toBe('Remove command 1')
    expect(buttons[1].attributes('aria-label')).toBe('Remove command 2')
  })

  // A page with both a Pre-backup and a Post-backup CommandListEditor (every
  // caller of this component) would otherwise have two buttons both named
  // "Remove command 1" - the given accessible name needs to fold into the
  // remove/add buttons too, not just the row fields, to actually disambiguate
  // them for a screen reader or voice-control user. The add button's label
  // keeps "+ Add command" as an exact substring (WCAG 2.5.3 Label in Name) -
  // an aria-label that replaced the visible text outright broke the e2e test
  // that clicks it by its visible name.
  it('folds the given accessible name into the remove and add buttons', () => {
    const wrapper = renderWithPlugins(CommandListEditor, {
      props: {
        modelValue: [hookCommand('first'), hookCommand('second')],
        ariaLabel: 'Pre-backup commands',
        'onUpdate:modelValue': () => {},
      },
    })
    const buttons = wrapper.findAll('.btn-danger')
    expect(buttons[0].attributes('aria-label')).toBe('Remove Pre-backup commands 1')
    expect(buttons[1].attributes('aria-label')).toBe('Remove Pre-backup commands 2')
    expect(wrapper.find('.btn-ghost').attributes('aria-label')).toBe(
      '+ Add command (Pre-backup commands)',
    )
  })

  it('renders a timeout field alongside every command', () => {
    const wrapper = mount([hookCommand('first'), hookCommand('second', 900)])
    const timeouts = wrapper.findAll('input[type="number"]')
    expect(timeouts).toHaveLength(2)
    expect((timeouts[0].element as HTMLInputElement).value).toBe('')
    expect((timeouts[1].element as HTMLInputElement).value).toBe('900')
  })

  // The point of the field: an inherited timeout has to read as "inherits
  // that number", not as "no timeout", or the empty state looks like a bug.
  it("shows the schedule's timeout as the placeholder for an inherited one", () => {
    const wrapper = renderWithPlugins(CommandListEditor, {
      props: {
        modelValue: [hookCommand('first')],
        defaultTimeoutSeconds: 60,
        'onUpdate:modelValue': () => {},
      },
    })
    expect(wrapper.find('input[type="number"]').attributes('placeholder')).toBe('60')
  })

  it('falls back to naming the schedule default when none is given', () => {
    const wrapper = mount([hookCommand('first')])
    expect(wrapper.find('input[type="number"]').attributes('placeholder')).toBe('schedule default')
  })

  it('writes an edited timeout back on its own command', async () => {
    const wrapper = mount([hookCommand('first'), hookCommand('second')])
    await wrapper.findAll('input[type="number"]')[1].setValue('7200')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([
      [hookCommand('first'), hookCommand('second', 7200)],
    ])
  })

  // Clearing the field must mean "inherit the schedule's timeout" rather than
  // "run without one" - a hook with no timeout at all can stall a schedule
  // forever, which is exactly what the bound is for.
  it('treats a cleared timeout as inherited, not as unlimited', async () => {
    const wrapper = mount([hookCommand('first', 7200)])
    await wrapper.find('input[type="number"]').setValue('')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([[hookCommand('first', null)]])
  })

  it('caps the timeout field at the bound the server enforces', () => {
    const wrapper = mount([hookCommand('first')])
    expect(wrapper.find('input[type="number"]').attributes('max')).toBe(
      String(MAX_HOOK_COMMAND_TIMEOUT_SECONDS),
    )
  })

  it('numbers each row timeout field so they are distinguishable', () => {
    const wrapper = renderWithPlugins(CommandListEditor, {
      props: {
        modelValue: [hookCommand('first'), hookCommand('second')],
        ariaLabel: 'Pre-backup commands',
        'onUpdate:modelValue': () => {},
      },
    })
    const timeouts = wrapper.findAll('input[type="number"]')
    expect(timeouts[0].attributes('aria-label')).toBe('Pre-backup commands 1 timeout in seconds')
    expect(timeouts[1].attributes('aria-label')).toBe('Pre-backup commands 2 timeout in seconds')
  })

  // A row whose script is untouched but whose timeout changed still has to be
  // rebuilt from the incoming value, or the field would keep showing the old
  // number after an external reassignment.
  it('regenerates a row when only its timeout changes externally', async () => {
    const wrapper = mount([hookCommand('first')])
    await wrapper.setProps({ modelValue: [hookCommand('first', 120)] })
    expect((wrapper.find('input[type="number"]').element as HTMLInputElement).value).toBe('120')
  })

  it('leaves the add button unlabeled when no accessible name is given', () => {
    const wrapper = mount([hookCommand('first')])
    expect(wrapper.find('.btn-ghost').attributes('aria-label')).toBeUndefined()
  })
})
