// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import PaneRow from './PaneRow.vue'

function mount(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
  return renderWithPlugins(PaneRow, {
    props: { title: 'Wake host before backup', ...props },
    slots: { default: '<input class="input" />', ...slots },
  })
}

describe('PaneRow', () => {
  it('puts the setting on the left and its control in a track of its own', () => {
    const wrapper = mount()
    const row = wrapper.find('.pane-row')
    expect(row.exists()).toBe(true)
    expect(row.classes()).not.toContain('pane-row--stack')
    expect(row.find('.field-body .field-title').text()).toBe('Wake host before backup')
    // The control never escapes the track: outside it, it floats at the far
    // edge of a wide pane, which is what the row shape exists to stop.
    expect(row.find('.pane-row-control input.input').exists()).toBe(true)
  })

  it('stacks the control under the label for an editor', () => {
    expect(mount({ stack: true }).find('.pane-row').classes()).toContain('pane-row--stack')
  })

  it('labels its control when the title names one', () => {
    const wrapper = mount({ labelFor: 'wake-mac' })
    expect(wrapper.find('.field-title label').attributes('for')).toBe('wake-mac')
  })

  it('leaves the title unlabelled when no control owns it', () => {
    expect(mount().find('.field-title label').exists()).toBe(false)
  })

  it('renders a hint only when one is given', () => {
    expect(mount().find('.field-hint').exists()).toBe(false)
    expect(mount({ hint: 'Default: 7.' }).find('.field-hint').text()).toBe('Default: 7.')
  })

  it('discloses the help slot behind the hint button rather than rendering it', async () => {
    const wrapper = mount({ help: 'waking a host' }, { help: 'Sent before every backup.' })
    expect(wrapper.text()).not.toContain('Sent before every backup.')

    await wrapper.find('[aria-label="Help: waking a host"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('Sent before every backup.')
  })

  // `PaneRow` owns the `HelpHint`, so without this a caller has no way to flip
  // a popover that would run off the edge - which is how the refactor first
  // dropped the one row that used it.
  it('opens the help popover rightward by default and leftward on request', async () => {
    const start = mount({ help: 'waking a host' }, { help: 'Sent before every backup.' })
    await start.find('[aria-label="Help: waking a host"]').trigger('click')
    expect(start.find('.help-hint-pop').classes()).not.toContain('help-hint-pop--end')

    const end = mount(
      { help: 'waking a host', align: 'end' },
      { help: 'Sent before every backup.' },
    )
    await end.find('[aria-label="Help: waking a host"]').trigger('click')
    expect(end.find('.help-hint-pop').classes()).toContain('help-hint-pop--end')
  })

  // A row can be nothing but a control and its explanation. `help` had been
  // left out of the condition that mounts the label column, so such a row
  // would have dropped its `HelpHint` silently.
  it('still renders the help button on a row with no title or hint', async () => {
    const wrapper = mount({ title: undefined, help: 'waking a host' }, { help: 'Sent first.' })
    expect(wrapper.find('.field-body').exists()).toBe(true)

    await wrapper.find('[aria-label="Help: waking a host"]').trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain('Sent first.')
  })

  it('omits the help button when the row has nothing to disclose', () => {
    expect(mount().find('.help-hint').exists()).toBe(false)
  })

  // The gap between a label and its `?` was incidental template whitespace at
  // every call site, and Vue's condense mode drops that between two elements.
  it('keeps a space between the title and its help button', () => {
    const wrapper = mount({ title: 'Catch up missed runs', help: 'running once after an outage' })
    expect(wrapper.find('.field-title').text()).toBe('Catch up missed runs')
    // HTML collapses a run of whitespace, so what matters is that there is
    // some between the last word and the button, not how many characters.
    expect(wrapper.find('.field-title').html()).toMatch(
      /Catch up missed runs\s+<span class="help-hint"/,
    )
  })

  it('shares the title row with a secondary control through titleAside', () => {
    const wrapper = mount(
      {},
      { titleAside: '<button class="ref-toggle">Pattern Reference</button>' },
    )
    const title = wrapper.find('.field-title')
    expect(title.classes()).toContain('field-label-row')
    expect(title.find('.ref-toggle').exists()).toBe(true)
  })

  it('leaves the title a plain block when nothing shares its row', () => {
    expect(mount().find('.field-title').classes()).not.toContain('field-label-row')
  })

  it('adds to the title itself through titleExtra', () => {
    const wrapper = mount(
      { title: 'Repositories' },
      { titleExtra: '<span class="required">*</span>' },
    )
    expect(wrapper.find('.field-title .required').exists()).toBe(true)
  })
})
