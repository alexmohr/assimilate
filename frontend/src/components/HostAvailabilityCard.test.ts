// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import HostAvailabilityCard from './HostAvailabilityCard.vue'
import type { HostAvailabilityApi } from '../api/availability'
import type { HostAvailabilityResponse } from '../types/generated'

const REPO_AVAILABILITY: HostAvailabilityResponse = {
  intermittent: true,
  catch_up_recheck_minutes: 15,
  catch_up_give_up_minutes: 3 * 24 * 60,
  waiting: [
    {
      schedule_id: 4,
      schedule_name: 'Nightly servers',
      pending_for: '2026-09-22T02:00:00Z',
      last_probe_at: '2026-09-22T03:06:00Z',
      next_probe_at: '2026-09-22T03:21:00Z',
      give_up_at: '2026-09-25T02:00:00Z',
    },
  ],
}

const AGENT_AVAILABILITY: HostAvailabilityResponse = {
  intermittent: true,
  catch_up_recheck_minutes: null,
  catch_up_give_up_minutes: 0,
  waiting: [
    {
      schedule_id: 9,
      schedule_name: 'Nightly workstations',
      pending_for: '2026-09-22T02:00:00Z',
      last_probe_at: null,
      next_probe_at: null,
      give_up_at: null,
    },
  ],
}

/** A repository's api has `check`; an agent's does not - it can only be waited for. */
function repoApi(initial: HostAvailabilityResponse = REPO_AVAILABILITY) {
  return {
    load: vi.fn().mockResolvedValue(initial),
    save: vi.fn().mockImplementation(async (data) => ({ ...initial, ...data, waiting: [] })),
    check: vi.fn().mockResolvedValue({
      probed: 1,
      reachable: 1,
      started: 1,
      abandoned: 0,
      dropped: 0,
    }),
  } satisfies HostAvailabilityApi
}

function agentApi(initial: HostAvailabilityResponse = AGENT_AVAILABILITY) {
  return {
    load: vi.fn().mockResolvedValue(initial),
    save: vi.fn().mockImplementation(async (data) => ({ ...initial, ...data, waiting: [] })),
  } satisfies HostAvailabilityApi
}

async function mount(api: HostAvailabilityApi, canEdit = true) {
  const wrapper = renderWithPlugins(HostAvailabilityCard, { props: { api, canEdit } })
  await flushPromises()
  return wrapper
}

describe('HostAvailabilityCard', () => {
  it('says what an unreachable host means when it is not marked', async () => {
    const wrapper = await mount(repoApi({ ...REPO_AVAILABILITY, intermittent: false, waiting: [] }))
    expect(wrapper.text()).toContain('When the host is offline')
    expect(wrapper.text()).toContain('fails like any other error')
    // Nothing nested under a switch that is off.
    expect(wrapper.text()).not.toContain('Re-check every')
  })

  it('shows a marked repository with its interval and window', async () => {
    const wrapper = await mount(repoApi())
    expect(wrapper.text()).toContain('reported as skipped and run again once it is back')
    expect(wrapper.text()).toContain('15 minutes')
    expect(wrapper.text()).toContain('3 days')
  })

  it('reads a zero window as never giving up', async () => {
    const wrapper = await mount(repoApi({ ...REPO_AVAILABILITY, catch_up_give_up_minutes: 0 }))
    expect(wrapper.text()).toContain('Never')
  })

  it('lists what is waiting on a repository, with when it is next asked', async () => {
    const wrapper = await mount(repoApi())
    expect(wrapper.text()).toContain('Waiting to catch up')
    expect(wrapper.text()).toContain('Nightly servers')
    expect(wrapper.text()).toContain('last checked')
    expect(wrapper.text()).toContain('next check')
    expect(wrapper.text()).toContain('giving up')
    expect(wrapper.find('a').attributes('href')).toBe('/schedules/4')
  })

  it('checks a repository on demand and reloads what is waiting', async () => {
    const api = repoApi()
    const wrapper = await mount(api)
    await clickSectionButton(wrapper, 'Check now')
    expect(api.check).toHaveBeenCalledOnce()
    expect(api.load).toHaveBeenCalledTimes(2)
  })

  it('offers no check to a viewer who cannot edit', async () => {
    const wrapper = await mount(repoApi(), false)
    expect(wrapper.text()).toContain('Nightly servers')
    expect(wrapper.text()).not.toContain('Check now')
    expect(wrapper.text()).not.toContain('Edit')
  })

  /** An agent reconnects on its own: there is nothing to ask it, so no check. */
  it('has no re-check or check button for an agent', async () => {
    const wrapper = await mount(agentApi())
    expect(wrapper.text()).toContain('Nightly workstations')
    expect(wrapper.text()).toContain('runs when the agent reconnects')
    expect(wrapper.text()).not.toContain('Check now')
    expect(wrapper.text()).not.toContain('Re-check every')

    await startEditingSection(wrapper)
    expect(wrapper.find('#availability-recheck').exists()).toBe(false)
    expect(wrapper.find('#availability-give-up').exists()).toBe(true)
  })

  it('hides the nested fields until the host is marked', async () => {
    const wrapper = await mount(repoApi({ ...REPO_AVAILABILITY, intermittent: false, waiting: [] }))
    await startEditingSection(wrapper)
    expect(wrapper.find('#availability-recheck').exists()).toBe(false)

    await wrapper.find('button[role="switch"]').trigger('click')
    expect(wrapper.find('#availability-recheck').exists()).toBe(true)
    expect(wrapper.find('#availability-give-up').exists()).toBe(true)
  })

  it('saves the switch, the interval in minutes and a window typed in days', async () => {
    const api = repoApi({ ...REPO_AVAILABILITY, catch_up_give_up_minutes: 0 })
    const wrapper = await mount(api)
    await startEditingSection(wrapper)

    await wrapper.find('select[aria-label="Re-check interval unit"]').setValue('hours')
    await wrapper.find('#availability-recheck').setValue('2')
    // Zero is the "wait indefinitely" sentinel, so it renders as an empty field.
    expect((wrapper.find('#availability-give-up').element as HTMLInputElement).value).toBe('')
    await wrapper.find('select[aria-label="Give-up window unit"]').setValue('days')
    await wrapper.find('#availability-give-up').setValue('3')
    await clickSectionButton(wrapper, 'Save')

    expect(api.save).toHaveBeenCalledWith({
      intermittent: true,
      catch_up_recheck_minutes: 120,
      catch_up_give_up_minutes: 3 * 24 * 60,
    })
  })

  it('sends no interval for an agent', async () => {
    const api = agentApi()
    const wrapper = await mount(api)
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')
    expect(api.save).toHaveBeenCalledWith({
      intermittent: true,
      catch_up_recheck_minutes: undefined,
      catch_up_give_up_minutes: 0,
    })
  })

  it('keeps editing and shows the reason when saving fails', async () => {
    const api = repoApi()
    api.save.mockRejectedValueOnce(new Error('catch_up_give_up_minutes must leave room'))
    const wrapper = await mount(api)
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')
    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.find('#availability-give-up').exists()).toBe(true)
  })

  it('reports a load failure in place', async () => {
    const api = repoApi()
    api.load.mockRejectedValueOnce(new Error('boom'))
    const wrapper = await mount(api)
    expect(wrapper.find('.state-error').exists()).toBe(true)
  })
})
