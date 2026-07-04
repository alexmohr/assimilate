// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'

describe('FileChangePatternsEditor', () => {
  it('renders one row per pattern in modelValue', () => {
    const wrapper = mount(FileChangePatternsEditor, {
      props: { modelValue: '*/tmp/* ignore\n*/etc/config* fatal' },
    })
    expect(wrapper.findAll('.fcp-row')).toHaveLength(2)
    const inputs = wrapper.findAll('input[type="text"]')
    expect(inputs[0].element.value).toBe('*/tmp/*')
    expect(inputs[1].element.value).toBe('*/etc/config*')
    const selects = wrapper.findAll('select')
    expect(selects[0].element.value).toBe('ignore')
    expect(selects[1].element.value).toBe('fatal')
  })

  it('renders no rows for empty modelValue', () => {
    const wrapper = mount(FileChangePatternsEditor, { props: { modelValue: '' } })
    expect(wrapper.findAll('.fcp-row')).toHaveLength(0)
  })

  it('adds a warn row and emits serialized text when clicking add pattern', async () => {
    const wrapper = mount(FileChangePatternsEditor, { props: { modelValue: '' } })
    await wrapper.find('button.btn-ghost').trigger('click')
    expect(wrapper.findAll('.fcp-row')).toHaveLength(1)

    await wrapper.find('input[type="text"]').setValue('*/var/log*')
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted).toBeTruthy()
    expect(emitted!.at(-1)).toEqual(['*/var/log*'])
  })

  it('removes a row and emits the updated text', async () => {
    const wrapper = mount(FileChangePatternsEditor, {
      props: { modelValue: '*/tmp/* ignore\n*/etc/config* fatal' },
    })
    await wrapper.find('button.btn-danger').trigger('click')
    expect(wrapper.findAll('.fcp-row')).toHaveLength(1)
    const emitted = wrapper.emitted('update:modelValue')
    expect(emitted!.at(-1)).toEqual(['*/etc/config* fatal'])
  })

  it('re-parses rows when modelValue changes externally', async () => {
    const wrapper = mount(FileChangePatternsEditor, { props: { modelValue: '' } })
    await wrapper.setProps({ modelValue: '*/foo* warn' })
    expect(wrapper.findAll('.fcp-row')).toHaveLength(1)
    expect(wrapper.find('input[type="text"]').element.value).toBe('*/foo*')
  })

  it('renders default hint text when no hint slot is provided', () => {
    const wrapper = mount(FileChangePatternsEditor, { props: { modelValue: '' } })
    expect(wrapper.find('.field-hint').text()).toContain(
      'Unconfigured files still produce warnings.',
    )
  })

  it('renders custom hint text via the hint slot', () => {
    const wrapper = mount(FileChangePatternsEditor, {
      props: { modelValue: '' },
      slots: { hint: 'Custom hint text' },
    })
    expect(wrapper.find('.field-hint').text()).toBe('Custom hint text')
  })
})
