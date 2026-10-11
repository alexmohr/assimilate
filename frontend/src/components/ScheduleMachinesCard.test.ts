// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  mockApiClientRw,
  mockWebSocket,
  resetWsHandlers,
  wsHandlers,
} from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../composables/useWebSocket', () => mockWebSocket())

import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { scheduleMachine } from '../test-utils/machineFixtures'
import { apiClient } from '../api/client'
import ScheduleMachinesCard from './ScheduleMachinesCard.vue'
import type { ScheduleMachineResponse } from '../types/generated'

const AGENT = scheduleMachine({
  machine_id: 10,
  label: 'web-01',
  roles: ['agent'],
  power: { wake_enabled: false, has_mac_address: true, shutdown_after_backup: false },
  can_wait: false,
})
const NAS = scheduleMachine({ machine_id: 20, label: 'nas.lan', mode: 'wait' })
const SHARE = scheduleMachine({
  machine_id: 30,
  label: 'nas-media',
  roles: ['dependency'],
  power: { wake_enabled: false, has_mac_address: false, shutdown_after_backup: false },
  intermittent: true,
})

let machines: ScheduleMachineResponse[]

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleMachinesCard, {
    props: { scheduleId: 4, wakeOverride: 'host_default', canEdit: true, ...props },
  })
}

function rows(wrapper: ReturnType<typeof mount>): Record<string, string> {
  const values = wrapper.findAll('dd').map((d) => d.text())
  return Object.fromEntries(
    wrapper
      .findAll('dt')
      .map((d, i) => [`${d.get('.mono').text()} (${d.get('.host-role').text()})`, values[i] ?? '']),
  )
}

describe('ScheduleMachinesCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetWsHandlers()
    machines = [AGENT, NAS, SHARE]
    vi.mocked(apiClient.get).mockImplementation(async () => ({ data: { machines } }) as never)
  })

  it('lists every machine the schedule needs, with what its choice resolves to', async () => {
    const wrapper = mount()
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/4/machines')
    const shown = rows(wrapper)
    expect(shown['web-01 (agent)']).toContain('Not woken')
    expect(shown['nas.lan (repository host)']).toContain('Waits')
    expect(shown['nas.lan (repository host)']).toContain('Wait for it')
    expect(shown['nas.lan (repository host)']).toContain('its targets run last')
    // Not always online on its own, so it is waited for whatever the schedule says.
    expect(shown['nas-media (dependency)']).toContain('Waits')
    expect(shown['nas-media (dependency)']).toContain('not always online')
  })

  it('resolves against the schedule-wide setting still being edited', async () => {
    const wrapper = mount({ wakeOverride: 'enabled' })
    await flushPromises()

    const shown = rows(wrapper)
    expect(shown['web-01 (agent)']).toContain('Wakes')
    // An explicit wait is never woken, whatever the schedule-wide setting.
    expect(shown['nas.lan (repository host)']).toContain('Waits')
    // No MAC address to wake it with.
    expect(shown['nas-media (dependency)']).toContain('Cannot wake')
  })

  // An agent that is not connected has nothing to run the backup.
  it('offers Wait for it for every machine but an agent', async () => {
    const wrapper = mount()
    await flushPromises()
    await startEditingSection(wrapper)

    const groups = wrapper.findAll('[role="radiogroup"]')
    expect(groups).toHaveLength(3)
    expect(groups[0]!.findAll('[role="radio"]').map((o) => o.text())).toEqual([
      'Same as above',
      'Wake',
      "Don't wake",
    ])
    expect(groups[1]!.findAll('[role="radio"]').map((o) => o.text())).toEqual([
      'Same as above',
      'Wake',
      "Don't wake",
      'Wait for it',
    ])
  })

  it('saves only the machines given a choice of their own', async () => {
    const saved = [AGENT, { ...NAS, mode: null }, { ...SHARE, mode: 'wake' as const }]
    vi.mocked(apiClient.put).mockResolvedValue({ data: { machines: saved } } as never)
    const wrapper = mount()
    await flushPromises()
    await startEditingSection(wrapper)

    const segmented = wrapper.findAllComponents({ name: 'BaseSegmented' })
    await segmented[1]!.vm.$emit('update:modelValue', 'follow')
    await segmented[2]!.vm.$emit('update:modelValue', 'wake')
    await flushPromises()
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/schedules/4/machines', {
      machines: [{ machine_id: 30, mode: 'wake' }],
    })
    expect(wrapper.findAll('[role="radiogroup"]')).toHaveLength(0)
    expect(rows(wrapper)['nas-media (dependency)']).toContain('Cannot wake')
  })

  it('previews the effect of a choice before it is saved', async () => {
    const wrapper = mount()
    await flushPromises()
    await startEditingSection(wrapper)

    await wrapper
      .findAllComponents({ name: 'BaseSegmented' })[0]!
      .vm.$emit('update:modelValue', 'wake')
    await flushPromises()
    expect(wrapper.findAll('.pane-row')[0]!.text()).toContain('Wakes')
  })

  it('keeps the form open with the reason when saving fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('an agent cannot be waited for'))
    const wrapper = mount()
    await flushPromises()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')

    expect(wrapper.find('.form-error').text()).toContain('an agent cannot be waited for')
    expect(wrapper.findAll('[role="radiogroup"]')).toHaveLength(3)
  })

  it('says to pick targets first, and offers no Edit, when nothing is needed yet', async () => {
    machines = []
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.text()).toContain('Pick this job')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('offers no Edit to a viewer who may not change it', async () => {
    const wrapper = mount({ canEdit: false })
    await flushPromises()
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('says when every machine follows the setting above', async () => {
    machines = [AGENT]
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.text()).toContain('Every machine follows the setting above.')
  })

  it('shows a failed load as an error', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.get('.error-banner').text()).toContain('boom')
  })

  // Targets saved elsewhere on the page change which machines it needs; an
  // open form is left alone rather than reset under the user.
  it('reloads when something changes, unless the form is open', async () => {
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(apiClient.get).toHaveBeenCalledTimes(1)

    await startEditingSection(wrapper)
    vi.mocked(apiClient.get).mockClear()
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(apiClient.get).not.toHaveBeenCalled()
  })
})
