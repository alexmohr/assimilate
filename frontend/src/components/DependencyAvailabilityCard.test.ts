// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import * as shared from '../test-utils/sharedMocks'

vi.mock('../composables/useToast', () => shared.mockToast())
vi.mock('../composables/useWebSocket', () => shared.mockWebSocket())
vi.mock('../utils/error', () => shared.mockErrorUtilsPassthrough())
vi.mock('../api/client', () => shared.mockApiClientRw())

const toast = shared.toastSpies

import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { dependencyAvailability, dependencyWait } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyAvailabilityCard from './DependencyAvailabilityCard.vue'
import type { DependencyAvailabilityResponse, RepoCatchUpCheckResponse } from '../types/generated'

function serve(availability: DependencyAvailabilityResponse): void {
  vi.mocked(apiClient.get).mockResolvedValue({ data: availability })
}

async function render(canEdit = true, hostId = 3): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(DependencyAvailabilityCard, {
    props: { hostId, port: 445, canEdit },
  })
  await flushPromises()
  return wrapper
}

function outcome(partial: Partial<RepoCatchUpCheckResponse>): RepoCatchUpCheckResponse {
  return { probed: 1, reachable: 0, started: 0, abandoned: 0, dropped: 0, ...partial }
}

function checkNowButton(wrapper: ReturnType<typeof renderWithPlugins>) {
  return wrapper.findAll('button').find((b) => b.text() === 'Check now')
}

describe('DependencyAvailabilityCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    shared.resetToastSpies()
    shared.resetWsHandlers()
    serve(dependencyAvailability())
  })

  it('says what an unreachable dependency means when it is not marked', async () => {
    serve(dependencyAvailability({ intermittent: false, waiting: [] }))
    const wrapper = await render()
    expect(apiClient.get).toHaveBeenCalledWith('/dependency-hosts/3/availability')
    expect(wrapper.text()).toContain('When the host is offline')
    expect(wrapper.text()).toContain('fails like any other error')
    expect(wrapper.text()).not.toContain('Re-check every')
    expect(wrapper.text()).not.toContain('Waiting to catch up')
  })

  it('shows a marked dependency with its interval and window', async () => {
    const wrapper = await render()
    expect(wrapper.text()).toContain('run again once it answers')
    expect(wrapper.text()).toContain('15 minutes')
    expect(wrapper.text()).toContain('1 day')
  })

  it('reads a zero window as never giving up', async () => {
    serve(dependencyAvailability({ catch_up_give_up_minutes: 0 }))
    const wrapper = await render()
    expect(wrapper.text()).toContain('Never')
  })

  it('lists each run waiting on it, by schedule and agent', async () => {
    serve(
      dependencyAvailability({
        waiting: [
          dependencyWait(),
          dependencyWait({
            schedule_id: 5,
            schedule_name: 'Photos weekly',
            agent_id: 8,
            hostname: 'web-server-01',
            last_probe_at: null,
            next_probe_at: null,
            give_up_at: null,
            catching_up: true,
          }),
        ],
      }),
    )
    const wrapper = await render()
    const rows = wrapper.findAll('a.agent-row')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.attributes('href')).toBe('/schedules/4')
    expect(rows[0]!.text()).toContain('Media share nightly')
    expect(rows[0]!.text()).toContain('on media-store-01')
    const detail = rows[0]!.get('.agent-row-stats').text()
    expect(detail).toMatch(/^missed .+ · last checked .+ · next check .+ · giving up .+$/)

    expect(rows[1]!.attributes('href')).toBe('/schedules/5')
    expect(rows[1]!.text()).toContain('on web-server-01')
    expect(rows[1]!.get('.agent-row-stats').text()).toMatch(/^missed .+ · catching up now$/)
    expect(rows[1]!.get('.agent-row-stripe').classes()).toContain('agent-row-stripe--accent')
  })

  it('says a wait not yet asked about has not been checked', async () => {
    serve(
      dependencyAvailability({
        waiting: [dependencyWait({ last_probe_at: null, give_up_at: null })],
      }),
    )
    const wrapper = await render()
    const detail = wrapper.get('.agent-row-stats').text()
    expect(detail).toContain('not checked yet')
    expect(detail).not.toContain('giving up')
  })

  it.each([
    [outcome({ reachable: 1, started: 2 }), 'The dependency is back - catching up 2 runs'],
    [outcome({ reachable: 1, started: 1 }), 'The dependency is back - catching up 1 run'],
    [
      outcome({ reachable: 1 }),
      'The dependency is back, but each schedule runs again soon enough on its own',
    ],
    [outcome({}), 'The dependency is still not answering'],
    [outcome({ probed: 0 }), 'Nothing is waiting on this dependency'],
    [
      outcome({ probed: 0, abandoned: 2 }),
      'Stopped waiting - 2 runs past the window, reported as failed',
    ],
  ])('reports a check that found %o', async (found, text) => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: found })
    const wrapper = await render()
    await checkNowButton(wrapper)!.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/dependency-hosts/3/availability/check')
    expect(toast.success).toHaveBeenCalledWith(text)
    // Reloaded afterwards, so a caught-up wait leaves the list.
    expect(apiClient.get).toHaveBeenCalledTimes(2)
  })

  it('reports a check that failed', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('server busy'))
    const wrapper = await render()
    await checkNowButton(wrapper)!.trigger('click')
    await flushPromises()
    expect(toast.error).toHaveBeenCalledWith('server busy')
  })

  it('offers neither Edit nor Check now to a viewer', async () => {
    const wrapper = await render(false)
    expect(wrapper.text()).toContain('Waiting to catch up')
    expect(checkNowButton(wrapper)).toBeUndefined()
    expect(wrapper.findAll('button').some((b) => b.text() === 'Edit')).toBe(false)
  })

  it('saves the switch with its interval and window', async () => {
    serve(dependencyAvailability({ intermittent: false, waiting: [] }))
    vi.mocked(apiClient.put).mockResolvedValue({ data: dependencyAvailability({ waiting: [] }) })
    const wrapper = await render()
    await startEditingSection(wrapper)
    await wrapper.findComponent({ name: 'ToggleSwitch' }).vm.$emit('update:modelValue', true)
    await flushPromises()
    expect(wrapper.find('#dependency-give-up').exists()).toBe(true)
    await wrapper.get('#dependency-recheck').setValue(30)
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/availability', {
      intermittent: true,
      catch_up_recheck_minutes: 30,
      catch_up_give_up_minutes: 24 * 60,
    })
    expect(wrapper.emitted('saved')).toHaveLength(1)
    expect(wrapper.text()).toContain('Yes')
  })

  it('keeps editing and shows why a save failed', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('interval too short'))
    const wrapper = await render()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')
    expect(wrapper.get('.form-error').text()).toContain('interval too short')
    expect(wrapper.emitted('saved')).toBeUndefined()
  })

  it('shows a failed load in place of the section', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('not allowed'))
    const wrapper = await render()
    expect(wrapper.get('.state-error').text()).toBe('not allowed')
  })

  it('reloads when something changes, and when it is pointed at another dependency', async () => {
    const wrapper = await render()
    serve(dependencyAvailability({ waiting: [] }))
    shared.wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(wrapper.text()).not.toContain('Waiting to catch up')

    await wrapper.setProps({ hostId: 4 })
    await flushPromises()
    expect(apiClient.get).toHaveBeenLastCalledWith('/dependency-hosts/4/availability')
  })

  it('discards the edit on Cancel and shows the saved settings again', async () => {
    const wrapper = await render()
    await startEditingSection(wrapper)
    await wrapper.findComponent({ name: 'ToggleSwitch' }).vm.$emit('update:modelValue', false)
    await flushPromises()
    expect(wrapper.find('#dependency-recheck').exists()).toBe(false)

    await clickSectionButton(wrapper, 'Cancel')

    expect(apiClient.put).not.toHaveBeenCalled()
    expect(wrapper.findComponent({ name: 'ToggleSwitch' }).exists()).toBe(false)
    expect(wrapper.text()).toContain('Re-check every')
    expect(wrapper.text()).toContain('Yes')
    expect(wrapper.findAll('button').some((b) => b.text().trim() === 'Edit')).toBe(true)
  })

  it('saves a changed give-up window', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: dependencyAvailability() })
    const wrapper = await render()
    await startEditingSection(wrapper)
    await wrapper.get('#dependency-give-up').setValue(3)
    expect(wrapper.text()).toContain('Reported as a failed backup after 3 days.')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith('/dependency-hosts/3/availability', {
      intermittent: true,
      catch_up_recheck_minutes: 15,
      catch_up_give_up_minutes: 3 * 24 * 60,
    })
  })

  it('explains the switch, the re-check and the give-up window behind their help buttons', async () => {
    const wrapper = await render()
    await startEditingSection(wrapper)

    await wrapper.get('[aria-label="Help: what an unreachable dependency means"]').trigger('click')
    const switchHelp = wrapper.get('.help-hint-pop').text()
    expect(switchHelp).toContain('For a NAS that sleeps')
    expect(switchHelp).toContain('reported as skipped and run once as soon as it answers')
    expect(switchHelp).toContain('a machine that does not answer is a failed backup')

    await wrapper.get('[aria-label="Help: asking a dependency that was away"]').trigger('click')
    expect(wrapper.text()).toContain('Assimilate checks port 445 on this interval.')

    await wrapper
      .get('[aria-label="Help: bounding how long a run waits for a dependency"]')
      .trigger('click')
    expect(wrapper.text()).toContain('A run waiting on this machine is abandoned')
    expect(wrapper.text()).toContain('Mark as failed after, which counts missed runs')
  })
})
