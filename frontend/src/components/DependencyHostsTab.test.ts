// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  mockApiClient,
  mockWebSocket,
  resetWsHandlers,
  wsHandlers,
} from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClient())
vi.mock('../composables/useWebSocket', () => mockWebSocket())

import { renderWithPlugins } from '../test-utils'
import { dependencyHost } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyHostsTab from './DependencyHostsTab.vue'
import type { DependencyHostResponse, MachineWaitResponse } from '../types/generated'

/** Answers the list, and each machine with the waits given for it. */
function listing(
  hosts: DependencyHostResponse[],
  machineWaits: Record<number, MachineWaitResponse[]> = {},
): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    const machine = /^\/machines\/(\d+)$/.exec(url)
    if (machine) {
      const id = Number(machine[1])
      const waiting = machineWaits[id]
      return waiting
        ? Promise.resolve({ data: { id, waiting } })
        : Promise.reject(new Error('no machine'))
    }
    return Promise.resolve({ data: hosts })
  })
}

function wait(overrides: Partial<MachineWaitResponse> = {}): MachineWaitResponse {
  return {
    role: 'dependency',
    schedule_id: 7,
    schedule_name: 'Media share nightly',
    hostname: 'media-store-01',
    repo_id: null,
    repo_name: null,
    dependency_name: 'nas-media',
    pending_for: new Date().toISOString(),
    last_probe_at: null,
    next_probe_at: null,
    give_up_at: null,
    ...overrides,
  }
}

async function render(isAdmin = true): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(DependencyHostsTab, { props: { isAdmin } })
  await flushPromises()
  return wrapper
}

function names(wrapper: ReturnType<typeof renderWithPlugins>): string[] {
  return wrapper.findAll('[data-testid="dependency-row"] .list-row-name').map((n) => n.text())
}

describe('DependencyHostsTab', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    resetWsHandlers()
  })

  it('renders each dependency as one row that links to its page', async () => {
    listing([
      dependencyHost({
        intermittent: true,
        last_check_reachable: false,
        last_checked_at: new Date(Date.now() - 4 * 60_000).toISOString(),
        repo_host: { id: 9, ssh_host: 'nas-01' },
        wake_enabled: true,
      }),
    ])
    const wrapper = await render()

    const row = wrapper.get('[data-testid="dependency-row"]')
    expect(row.get('a.list-row-name').attributes('href')).toBe('/dependency-hosts/3')
    expect(row.get('.list-row-name').text()).toBe('nas-media')
    expect(row.get('.status-dot').classes()).toContain('status-dot--warning')
    expect(row.get('[data-testid="dependency-address"]').text()).toBe('SMB · nas-media.lan:445')
    expect(row.text()).toContain('Same machine as nas-01')
    const chips = row.findAll('[data-testid="dependency-availability"] .badge').map((b) => b.text())
    expect(chips).toEqual(['Not always online', 'Wake on demand'])
    const usedBy = row.findAll('.dependency-used-link')
    expect(usedBy.map((l) => l.text())).toEqual(['2 schedules', '1 agent default'])
    for (const link of usedBy) {
      expect(link.attributes('href')).toBe('/dependency-hosts/3?section=used-by')
    }
    const checked = row.get('.dependency-checked')
    expect(checked.text()).toContain('4m ago')
    expect(checked.classes()).toContain('dependency-checked--warning')
    expect(wrapper.find('.dependency-waiting').exists()).toBe(false)
  })

  it('opens the dependency when the row itself is clicked', async () => {
    listing([dependencyHost()])
    const wrapper = await render()
    const push = vi.spyOn(wrapper.vm.$router, 'push')
    await wrapper.get('[data-testid="dependency-row"]').trigger('click')
    expect(push).toHaveBeenCalledWith('/dependency-hosts/3')
  })

  it.each([
    [true, 'badge--success', 'Answering'],
    [false, 'badge--warning', 'Not answering'],
    [null, 'badge--neutral', 'Not checked yet'],
  ])('reads a last check of %s as %s "%s"', async (reachable, tone, label) => {
    listing([dependencyHost({ last_check_reachable: reachable })])
    const wrapper = await render()
    const badge = wrapper.get('.dependency-name-line .badge')
    expect(badge.classes()).toContain(tone)
    expect(badge.text()).toBe(label)
  })

  it('names the protocol from the port, and says when it was never checked', async () => {
    listing([
      dependencyHost({ id: 1, name: 'a', port: 2049 }),
      dependencyHost({ id: 2, name: 'b', port: 22, wake_enabled: true }),
      dependencyHost({ id: 3, name: 'c', port: 8443, schedule_count: 0, agent_default_count: 0 }),
    ])
    const wrapper = await render()
    const protocols = wrapper
      .findAll('[data-testid="dependency-address"]')
      .map((f) => f.text().split(' · ')[0])
    expect(protocols).toEqual(['NFS', 'SSH', 'TCP'])
    expect(wrapper.text()).toContain('Always online')
    // Wake on an always-online machine is not "on demand".
    expect(
      wrapper.findAll('[data-testid="dependency-availability"] .badge').map((b) => b.text()),
    ).toContain('Wake')
    const never = wrapper.get('.dependency-checked')
    expect(never.text()).toContain('Never')
    expect(never.classes()).toContain('muted')
    expect(wrapper.text()).toContain('Nothing yet')
    expect(wrapper.text()).not.toContain('runs waiting')
    expect(wrapper.text()).not.toContain('Same machine as')
  })

  it('counts the hosts and the schedules that depend on them', async () => {
    listing([
      dependencyHost({ id: 1, name: 'a', schedule_count: 2 }),
      dependencyHost({ id: 2, name: 'b', schedule_count: 1 }),
    ])
    const wrapper = await render()
    expect(wrapper.get('.list-group-title').text()).toBe('All dependencies')
    expect(wrapper.get('.list-group-count').text().replace(/\s+/g, ' ')).toBe(
      '2 hosts · 3 schedules depend on them',
    )
  })

  it('lists the dependencies that need looking at first', async () => {
    listing([
      dependencyHost({ id: 1, name: 'answering' }),
      dependencyHost({ id: 2, name: 'unchecked', last_check_reachable: null }),
      dependencyHost({ id: 3, name: 'waited-on', waiting_count: 1 }),
      dependencyHost({ id: 4, name: 'silent', last_check_reachable: false }),
    ])
    const wrapper = await render()
    expect(names(wrapper)).toEqual(['silent', 'waited-on', 'unchecked', 'answering'])
  })

  it('offers counted filters only for what the list holds, and filters by them', async () => {
    listing([
      dependencyHost({ id: 1, name: 'ok' }),
      dependencyHost({ id: 2, name: 'silent', last_check_reachable: false }),
      dependencyHost({ id: 3, name: 'unchecked', last_check_reachable: null }),
    ])
    const wrapper = await render()
    const pills = (): string[] =>
      wrapper.findAll('.filter-pill').map((p) => p.text().replace(/\s+/g, ' '))
    expect(pills()).toEqual(['All · 3', 'Not answering · 1', 'Not checked · 1'])

    await wrapper.findAll('.filter-pill')[1].trigger('click')
    expect(names(wrapper)).toEqual(['silent'])
    expect(wrapper.findAll('.filter-pill')[1].attributes('aria-pressed')).toBe('true')

    await wrapper.findAll('.filter-pill')[2].trigger('click')
    expect(names(wrapper)).toEqual(['unchecked'])

    await wrapper.findAll('.filter-pill')[0].trigger('click')
    expect(names(wrapper)).toHaveLength(3)
  })

  it('searches by name and by address', async () => {
    listing([
      dependencyHost({ id: 1, name: 'nas-media', address: 'nas-media.lan' }),
      dependencyHost({ id: 2, name: 'files-01', address: '10.0.0.5' }),
    ])
    const wrapper = await render()
    const input = wrapper.get('input[type="search"]')

    await input.setValue('FILES')
    expect(names(wrapper)).toEqual(['files-01'])

    await input.setValue('10.0.0')
    expect(names(wrapper)).toEqual(['files-01'])

    await input.setValue('nothing-like-it')
    expect(names(wrapper)).toEqual([])
    expect(wrapper.text()).toContain('No dependency matches.')
  })

  it('names the runs waiting on a dependency under its row', async () => {
    const since = new Date()
    since.setHours(3, 5, 0, 0)
    listing([dependencyHost({ waiting_count: 1, machine_id: 30 })], {
      30: [
        wait({ pending_for: since.toISOString() }),
        // Another dependency on the same machine: not this row's wait.
        wait({ dependency_name: 'files-01', schedule_name: 'Other' }),
        wait({ role: 'repo_host', dependency_name: null, schedule_name: 'Repo wait' }),
      ],
    })
    const wrapper = await render()

    const waiting = wrapper.get('.dependency-waiting')
    expect(waiting.get('.dependency-waiting-count').text()).toBe('1 run waiting')
    const lines = waiting.findAll('.list-row-sub').map((l) => l.text())
    expect(lines).toHaveLength(1)
    expect(lines[0]).toMatch(/^Media share nightly on media-store-01 · waiting since 0?3:05$/)
    const link = waiting.get('a')
    expect(link.text()).toBe('View run')
    expect(link.attributes('href')).toBe('/dependency-hosts/3?section=power')
  })

  it('says when a waiting run gives up, and names an unnamed schedule', async () => {
    listing([dependencyHost({ waiting_count: 2 })], {
      30: [
        wait({ schedule_name: '', schedule_id: 42, give_up_at: new Date().toISOString() }),
        wait({ hostname: null }),
      ],
    })
    const wrapper = await render()
    const lines = wrapper.findAll('.dependency-waiting .list-row-sub').map((l) => l.text())
    expect(lines[0]).toMatch(/^Schedule #42 on media-store-01 · waiting since .+ · gives up .+$/)
    expect(lines[1]).toMatch(/^Media share nightly · waiting since/)
    expect(wrapper.get('.dependency-waiting a').text()).toBe('View runs')
    expect(wrapper.get('.dependency-waiting-count').text()).toBe('2 runs waiting')
  })

  it('still shows the waiting count when its machine cannot be read', async () => {
    listing([dependencyHost({ waiting_count: 3 })])
    const wrapper = await render()
    expect(wrapper.get('.dependency-waiting-count').text()).toBe('3 runs waiting')
    expect(wrapper.findAll('.dependency-waiting .list-row-sub')).toHaveLength(0)
  })

  it('offers New on the empty state to admins only', async () => {
    listing([])
    const admin = await render(true)
    expect(admin.text()).toContain('No dependencies yet')
    await admin.get('.empty-action').trigger('click')
    expect(admin.emitted('new')).toHaveLength(1)

    const viewer = await render(false)
    expect(viewer.text()).toContain('No dependencies yet')
    expect(viewer.find('.empty-action').exists()).toBe(false)
  })

  it('shows a failed first load as an error', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('nope'))
    const wrapper = await render()
    expect(wrapper.find('.error-banner').exists()).toBe(true)
  })

  it('reloads when something changes, keeping the rows if that fails', async () => {
    listing([dependencyHost()])
    const wrapper = await render()

    listing([dependencyHost(), dependencyHost({ id: 4, name: 'files-01' })])
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(wrapper.findAll('[data-testid="dependency-row"]')).toHaveLength(2)

    vi.mocked(apiClient.get).mockRejectedValue(new Error('gone'))
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(wrapper.findAll('[data-testid="dependency-row"]')).toHaveLength(2)
    expect(wrapper.find('.error-banner').exists()).toBe(false)
  })
})
