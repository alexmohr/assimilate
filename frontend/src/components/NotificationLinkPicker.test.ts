// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import NotificationLinkPicker from './NotificationLinkPicker.vue'
import { NOTIFICATION_LINK_OPTIONS, type NotificationLink } from '../utils/notificationLink'

function render(link: NotificationLink, idPrefix = 'pick') {
  // A v-model parent: each emitted value is fed back in, as the wizard and
  // the channel page do.
  const wrapper = mount(NotificationLinkPicker, {
    props: {
      modelValue: link,
      idPrefix,
      'onUpdate:modelValue': async (next: NotificationLink): Promise<void> => {
        await wrapper.setProps({ modelValue: next })
      },
    },
  })
  return wrapper
}

function radios(wrapper: ReturnType<typeof render>) {
  return wrapper.findAll<HTMLInputElement>('input[type="radio"]')
}

function emitted(wrapper: ReturnType<typeof render>): NotificationLink[] {
  return (wrapper.emitted('update:modelValue') ?? []).map((args) => args[0] as NotificationLink)
}

describe('NotificationLinkPicker', () => {
  it('is one labelled radio group with a choice per option', () => {
    const wrapper = render({ target: 'automatic' })
    const group = wrapper.find('[role="radiogroup"]')
    expect(group.attributes('aria-label')).toBe('Where notifications link to')
    expect(radios(wrapper).map((r) => r.element.value)).toEqual(
      NOTIFICATION_LINK_OPTIONS.map((o) => o.value),
    )
    const labels = wrapper.findAll('.link-option-label').map((l) => l.text())
    expect(labels).toEqual(NOTIFICATION_LINK_OPTIONS.map((o) => o.label))
    const hints = wrapper.findAll('.link-option-hint').map((l) => l.text())
    expect(hints).toEqual(NOTIFICATION_LINK_OPTIONS.map((o) => o.hint))
  })

  it('checks the current target, and only that one', () => {
    const wrapper = render({ target: 'schedule' })
    const checked = radios(wrapper).filter((r) => r.element.checked)
    expect(checked.map((r) => r.element.value)).toEqual(['schedule'])
    expect(wrapper.find('.link-option--checked .link-option-label').text()).toBe('Schedule')
  })

  it('keeps two pickers on one page apart by their id prefix', () => {
    const wrapper = render({ target: 'automatic' }, 'channel-link')
    const [first] = radios(wrapper)
    expect(first.attributes('id')).toBe('channel-link-automatic')
    expect(first.attributes('name')).toBe('channel-link-target')
  })

  it('emits the newly picked target', async () => {
    const wrapper = render({ target: 'automatic' })
    await radios(wrapper)
      .find((r) => r.element.value === 'dashboard')!
      .setValue(true)
    expect(emitted(wrapper)).toEqual([{ target: 'dashboard', url: undefined }])
  })

  it('asks for an address only when your own is picked', async () => {
    const wrapper = render({ target: 'automatic' })
    expect(wrapper.find('#pick-url').exists()).toBe(false)

    await radios(wrapper)
      .find((r) => r.element.value === 'custom')!
      .setValue(true)

    const url = wrapper.find<HTMLInputElement>('#pick-url')
    expect(url.exists()).toBe(true)
    expect(url.element.value).toBe('')
    expect(wrapper.find('label[for="pick-url"]').text()).toBe('Address')
  })

  it('shows the saved address and emits each edit to it', async () => {
    const wrapper = render({ target: 'custom', url: 'https://old.example.com' })
    const url = wrapper.find<HTMLInputElement>('#pick-url')
    expect(url.element.value).toBe('https://old.example.com')

    await url.setValue('https://status.example.com')
    expect(emitted(wrapper).at(-1)).toEqual({
      target: 'custom',
      url: 'https://status.example.com',
    })
  })

  it('keeps a typed address when stepping away from your own and back', async () => {
    const wrapper = render({ target: 'custom', url: 'https://status.example.com' })

    await radios(wrapper)
      .find((r) => r.element.value === 'none')!
      .setValue(true)
    expect(emitted(wrapper).at(-1)).toEqual({
      target: 'none',
      url: 'https://status.example.com',
    })
    expect(wrapper.find('#pick-url').exists()).toBe(false)

    await radios(wrapper)
      .find((r) => r.element.value === 'custom')!
      .setValue(true)
    expect(wrapper.find<HTMLInputElement>('#pick-url').element.value).toBe(
      'https://status.example.com',
    )
  })
})
