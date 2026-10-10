// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import NotificationEventToggles from './NotificationEventToggles.vue'
import type { EventType } from '../types/generated'

const EVENTS: EventType[] = ['backup_success', 'backup_failed']

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(NotificationEventToggles, {
    props: {
      eventTypes: EVENTS,
      labelFor: (et: EventType) => `label:${et}`,
      isEnabled: (et: EventType) => et === 'backup_failed',
      ...props,
    },
  })
}

function switches(wrapper: ReturnType<typeof mount>): HTMLButtonElement[] {
  return wrapper.findAll<HTMLButtonElement>('.event-item [role="switch"]').map((b) => b.element)
}

describe('NotificationEventToggles', () => {
  it('renders one labelled switch per event type', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.event-label').map((l) => l.text())).toEqual([
      'label:backup_success',
      'label:backup_failed',
    ])
  })

  it('reflects which events are on', () => {
    expect(switches(mount()).map((s) => s.getAttribute('aria-checked'))).toEqual(['false', 'true'])
  })

  it('leaves every switch enabled unless told otherwise', () => {
    expect(switches(mount()).map((s) => s.disabled)).toEqual([false, false])
  })

  it('locks the switches the caller marks busy', () => {
    const wrapper = mount({ isDisabled: (et: EventType) => et === 'backup_success' })
    expect(switches(wrapper).map((s) => s.disabled)).toEqual([true, false])
  })

  it('emits the event type of a flipped switch', async () => {
    const wrapper = mount()
    await wrapper.findAll('.event-item [role="switch"]')[0].trigger('click')
    expect(wrapper.emitted('toggle')).toEqual([['backup_success']])
  })
})
