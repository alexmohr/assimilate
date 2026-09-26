// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import { rowsFromSaved, type WebhookHeaderRow } from '../utils/webhookHeaders'
import WebhookHeadersEditor from './WebhookHeadersEditor.vue'

function mount(modelValue: WebhookHeaderRow[], needsReentry: string[] = []) {
  return renderWithPlugins(WebhookHeadersEditor, {
    props: { modelValue, needsReentry, 'onUpdate:modelValue': () => {} },
  })
}

describe('WebhookHeadersEditor', () => {
  it('renders a name and a masked value field per header', () => {
    const wrapper = mount(rowsFromSaved([{ name: 'Authorization', has_value: true }]))
    const [name, value] = wrapper.findAll('input')
    expect((name.element as HTMLInputElement).value).toBe('Authorization')
    expect(value.attributes('type')).toBe('password')
    expect(value.attributes('placeholder')).toMatch(/leave blank to keep it/)
  })

  it('does not promise to keep a value that was never saved', () => {
    const wrapper = mount(rowsFromSaved([{ name: 'X-Empty', has_value: false }]))
    expect(wrapper.findAll('input')[1].attributes('placeholder')).toBe('Value')
  })

  it('adds an empty row', async () => {
    const wrapper = mount([])
    await wrapper.find('[data-testid="webhook-add-header"]').trigger('click')
    const emitted = wrapper.emitted('update:modelValue')?.at(-1)?.[0] as WebhookHeaderRow[]
    expect(emitted.map((r) => [r.name, r.value, r.savedName])).toEqual([['', '', null]])
  })

  it('removes the row whose button was pressed', async () => {
    const rows = rowsFromSaved([
      { name: 'A', has_value: true },
      { name: 'B', has_value: true },
    ])
    const wrapper = mount(rows)
    await wrapper.find('[aria-label="Remove header 1"]').trigger('click')
    const emitted = wrapper.emitted('update:modelValue')?.at(-1)?.[0] as WebhookHeaderRow[]
    expect(emitted.map((r) => r.name)).toEqual(['B'])
  })

  it('says which saved values need typing again', () => {
    expect(mount([]).find('[data-testid="webhook-header-reentry"]').exists()).toBe(false)
    const warning = mount([], ['Authorization']).find('[data-testid="webhook-header-reentry"]')
    expect(warning.text()).toContain('Authorization')
  })
})
