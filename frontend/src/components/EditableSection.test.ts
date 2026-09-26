// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import EditableSection from './EditableSection.vue'

const SLOTS = {
  view: '<span class="view-body">read only</span>',
  hint: 'what this setting does',
  edit: '<span class="edit-body">form</span>',
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(EditableSection, {
    props: {
      lede: 'What a schedule uses when it sets none of its own.',
      ledeLabel: 'backup defaults',
      hintLabel: 'command inheritance',
      editing: false,
      ...props,
    },
    slots: SLOTS,
  })
}

/** Buttons that are not the lede/hint `HelpHint` disclosures. */
function editingButtons(wrapper: ReturnType<typeof mount>) {
  return wrapper.findAll('button').filter((b) => !b.classes('help-hint-btn'))
}

describe('EditableSection', () => {
  it('shows the view slot and discloses the lede and hint behind their own HelpHints', async () => {
    const wrapper = mount()
    expect(wrapper.find('.view-body').exists()).toBe(true)
    expect(wrapper.find('.edit-body').exists()).toBe(false)

    await wrapper.find('[aria-label="Help: backup defaults"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toBe(
      'What a schedule uses when it sets none of its own.',
    )

    await wrapper.find('[aria-label="Help: command inheritance"]').trigger('click')
    const hintPop = wrapper
      .findAll('.help-hint-pop')
      .find((p) => p.text() === 'what this setting does')
    expect(hintPop).toBeDefined()
  })

  it('shows the edit slot and hides the hint when editing', () => {
    const wrapper = mount({ editing: true })
    expect(wrapper.find('.edit-body').exists()).toBe(true)
    expect(wrapper.find('.view-body').exists()).toBe(false)
    expect(wrapper.find('[aria-label="Help: command inheritance"]').exists()).toBe(false)
  })

  it('omits the hint HelpHint entirely when no hint slot is given', () => {
    const wrapper = renderWithPlugins(EditableSection, {
      props: { editing: false },
      slots: { view: '<span />' },
    })
    expect(wrapper.find('[aria-label="Help: this section"]').exists()).toBe(false)
  })

  it('renders no pane head at all when there is neither a lede nor an Edit button', () => {
    const wrapper = renderWithPlugins(EditableSection, {
      props: { editing: false },
      slots: { view: '<span />' },
    })
    expect(wrapper.find('.pane-head').exists()).toBe(false)
  })

  it('puts a lone Edit button at the trailing edge of its row', () => {
    const bare = renderWithPlugins(EditableSection, {
      props: { editing: false, canEdit: true },
      slots: { view: '<span />' },
    })
    expect(bare.find('.pane-head').classes()).toContain('pane-head--end')

    // Beside a lede the row spreads the two apart as before.
    expect(mount({ canEdit: true }).find('.pane-head').classes()).not.toContain('pane-head--end')
  })

  // One of several groups in a pane: the label and the Edit button share a
  // row, rather than the button sitting on a line of its own under the label.
  it("puts a group label on the Edit button's row", () => {
    const wrapper = renderWithPlugins(EditableSection, {
      props: { editing: false, canEdit: true, label: 'When the host is offline' },
      slots: { view: '<span />' },
    })
    const head = wrapper.find('.pane-section-head')
    expect(head.find('.group-label').text()).toBe('When the host is offline')
    expect(head.find('button').text()).toBe('Edit')
    expect(wrapper.find('.pane-head').exists()).toBe(false)
  })

  it('keeps the label while editing, without the Edit button', () => {
    const wrapper = renderWithPlugins(EditableSection, {
      props: { editing: true, canEdit: true, label: 'Address' },
      slots: { edit: '<span class="edit-body" />' },
    })
    expect(wrapper.find('.pane-section-head .group-label').text()).toBe('Address')
    expect(wrapper.findAll('button').map((b) => b.text())).not.toContain('Edit')
  })

  it('hides the Edit button unless the caller says the section is editable', () => {
    expect(editingButtons(mount())).toHaveLength(0)
    const withEdit = editingButtons(mount({ canEdit: true }))
    expect(withEdit).toHaveLength(1)
    expect(withEdit[0].text()).toBe('Edit')
  })

  it('emits edit, cancel and save rather than owning the state itself', async () => {
    const readOnly = mount({ canEdit: true })
    await editingButtons(readOnly)[0].trigger('click')
    expect(readOnly.emitted('edit')).toHaveLength(1)

    const editing = mount({ editing: true })
    const buttons = editingButtons(editing)
    await buttons[0].trigger('click')
    await buttons[1].trigger('click')
    expect(editing.emitted('cancel')).toHaveLength(1)
    expect(editing.emitted('save')).toHaveLength(1)
  })

  it('shows the error and disables both editing buttons while saving, leaving the lede HelpHint usable', () => {
    const wrapper = mount({ editing: true, saving: true, error: 'SSH unreachable' })
    expect(wrapper.find('.form-error').text()).toBe('SSH unreachable')

    const buttons = editingButtons(wrapper)
    expect(buttons).toHaveLength(2)
    for (const button of buttons) {
      expect(button.attributes('disabled')).toBeDefined()
    }
    expect(buttons[1].text()).toBe('Saving...')

    // Reading the explanation does not require the ability to save.
    expect(wrapper.find('.help-hint-btn').attributes('disabled')).toBeUndefined()
  })
})
