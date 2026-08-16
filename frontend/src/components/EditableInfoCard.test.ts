// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import EditableInfoCard from './EditableInfoCard.vue'

const SLOTS = {
  view: '<span class="view-body">read only</span>',
  hint: 'what this setting does',
  edit: '<span class="edit-body">form</span>',
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(EditableInfoCard, {
    props: { title: 'Default Backup Paths', editing: false, ...props },
    slots: SLOTS,
  })
}

describe('EditableInfoCard', () => {
  it('shows the view slot and the hint when not editing', () => {
    const wrapper = mount()
    expect(wrapper.find('.info-title').text()).toBe('Default Backup Paths')
    expect(wrapper.find('.view-body').exists()).toBe(true)
    expect(wrapper.find('.field-hint').text()).toBe('what this setting does')
    expect(wrapper.find('.edit-body').exists()).toBe(false)
  })

  it('shows the edit slot and hides the hint when editing', () => {
    const wrapper = mount({ editing: true })
    expect(wrapper.find('.edit-body').exists()).toBe(true)
    expect(wrapper.find('.view-body').exists()).toBe(false)
    expect(wrapper.find('.field-hint').exists()).toBe(false)
  })

  it('omits the hint entirely when no hint slot is given', () => {
    const wrapper = renderWithPlugins(EditableInfoCard, {
      props: { title: 'Options', editing: false },
      slots: { view: '<span />' },
    })
    expect(wrapper.find('.field-hint').exists()).toBe(false)
  })

  it('hides the Edit button unless the caller says the card is editable', () => {
    expect(mount().find('button').exists()).toBe(false)
    expect(mount({ canEdit: true }).find('button').text()).toBe('Edit')
  })

  it('emits edit, cancel and save rather than owning the state itself', async () => {
    const readOnly = mount({ canEdit: true })
    await readOnly.find('button').trigger('click')
    expect(readOnly.emitted('edit')).toHaveLength(1)

    const editing = mount({ editing: true })
    const buttons = editing.findAll('button')
    await buttons[0].trigger('click')
    await buttons[1].trigger('click')
    expect(editing.emitted('cancel')).toHaveLength(1)
    expect(editing.emitted('save')).toHaveLength(1)
  })

  it('shows the error and disables both buttons while saving', () => {
    const wrapper = mount({ editing: true, saving: true, error: 'SSH unreachable' })
    expect(wrapper.find('.form-error').text()).toBe('SSH unreachable')
    for (const button of wrapper.findAll('button')) {
      expect(button.attributes('disabled')).toBeDefined()
    }
    expect(wrapper.findAll('button')[1].text()).toBe('Saving...')
  })
})
