// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
import { mockToast } from '../test-utils/sharedMocks'
import { machine, machineWait } from '../test-utils/machineFixtures'
import { apiClient } from '../api/client'
import { useToast } from '../composables/useToast'
import MachineAvailabilitySection from './MachineAvailabilitySection.vue'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn(), post: vi.fn(), get: vi.fn() },
}))
vi.mock('../composables/useToast', () => mockToast())

const SLEEPING = machine({
  availability: {
    intermittent: true,
    catch_up_recheck_minutes: 30,
    catch_up_give_up_minutes: 2880,
  },
  waiting: [
    machineWait({ last_probe_at: '2026-09-26T02:15:00Z', give_up_at: '2026-09-28T02:00:00Z' }),
  ],
})

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(MachineAvailabilitySection, {
    props: { machine: SLEEPING, canEdit: true, ...props },
  })
}

function button(wrapper: ReturnType<typeof mount>, label: string) {
  return wrapper.findAll('button').find((b) => b.text().trim() === label)
}

describe('MachineAvailabilitySection', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.put).mockResolvedValue({ data: SLEEPING } as never)
  })

  it('shows the interval and the window of a machine that is not always online', () => {
    const text = mount().text()
    expect(text).toContain('Host is not always online')
    expect(text).toContain('Yes')
    expect(text).toContain('30 minutes')
    expect(text).toContain('2 days')
    expect(text).toContain('reported as skipped')
  })

  // Off still lets a schedule choose to wait for it.
  it('says an unmarked machine fails a run unless its schedule waits for it', () => {
    const text = mount({ machine: machine() }).text()
    expect(text).toContain('fails like any other error, unless its schedule waits for it')
  })

  it('lists every run waiting on it, with when it is asked and given up on', () => {
    const wrapper = mount()
    const row = wrapper.get('.agent-row')
    expect(row.get('a').attributes('href')).toBe('/schedules/7')
    expect(row.text()).toContain('Nightly servers')
    const detail = row.get('.agent-row-stats').text()
    expect(detail).toContain('server-daily')
    expect(detail).toContain('missed')
    expect(detail).toContain('last checked')
    expect(detail).toContain('next check')
    expect(detail).toContain('giving up')
  })

  it('names the dependency and the agent a dependency wait is for', () => {
    const wrapper = mount({
      machine: machine({
        waiting: [
          machineWait({
            role: 'dependency',
            repo_id: null,
            repo_name: null,
            hostname: 'media-store-01',
            dependency_name: 'nas-media',
          }),
        ],
      }),
    })
    expect(wrapper.get('.agent-row-stats').text()).toContain('nas-media for media-store-01')
  })

  // An agent reconnects on its own: nothing asks it, so there is no interval.
  it('runs an agent wait when the agent reconnects, and asks no interval of an agent alone', async () => {
    const agentOnly = machine({
      roles: {
        agent: { id: 1, hostname: 'laptop-01', domain: null },
        repo_host: null,
        dependencies: [],
      },
      availability: {
        intermittent: true,
        catch_up_recheck_minutes: 15,
        catch_up_give_up_minutes: 0,
      },
      waiting: [
        machineWait({
          role: 'agent',
          hostname: 'laptop-01',
          repo_id: null,
          repo_name: null,
          next_probe_at: null,
        }),
      ],
    })
    const wrapper = mount({ machine: agentOnly })
    expect(wrapper.text()).not.toContain('Re-check every')
    expect(wrapper.get('.agent-row-stats').text()).toContain('runs when the agent reconnects')

    await startEditingSection(wrapper)
    expect(wrapper.find('#availability-recheck').exists()).toBe(false)
    expect(wrapper.find('#availability-give-up').exists()).toBe(true)
  })

  it('saves the switch, the interval and the window to the machine', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#availability-recheck').setValue('45')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/machines/20/availability', {
      intermittent: true,
      catch_up_recheck_minutes: 45,
      catch_up_give_up_minutes: 2880,
    })
    expect(wrapper.emitted('saved')).toEqual([[SLEEPING]])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('out of range'))
    await expectSaveErrorKeepsEditing(mount(), 'out of range', '#availability-recheck')
  })

  it('closes a half-edited form when it is shown for another machine', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.setProps({ machine: { ...SLEEPING, id: 21 } })
    await flushPromises()
    expect(wrapper.find('#availability-recheck').exists()).toBe(false)
  })

  it.each([
    [
      { reachable: true, started: 2, abandoned: 0, dropped: 0 },
      'The machine is back - catching up 2 runs',
    ],
    [
      { reachable: false, started: 0, abandoned: 0, dropped: 0 },
      'The machine is still not answering',
    ],
  ])('checks the machine now and says what it found', async (outcome, text) => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: outcome } as never)
    const wrapper = mount()
    await button(wrapper, 'Check now')!.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/machines/20/check')
    expect(useToast().success).toHaveBeenCalledWith(text)
    expect(wrapper.emitted('checked')).toHaveLength(1)
  })

  it('reports a failed check as an error and still asks for a reload', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('timed out'))
    const wrapper = mount()
    await button(wrapper, 'Check now')!.trigger('click')
    await flushPromises()

    expect(useToast().error).toHaveBeenCalledWith('timed out')
    expect(wrapper.emitted('checked')).toHaveLength(1)
  })

  it('offers neither Edit nor Check now to a viewer who may not change it', () => {
    const wrapper = mount({ canEdit: false })
    expect(button(wrapper, 'Edit')).toBeUndefined()
    expect(button(wrapper, 'Check now')).toBeUndefined()
  })
})
