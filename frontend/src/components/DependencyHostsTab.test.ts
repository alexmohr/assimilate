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
import { dependencyHost, dependencyPower } from '../test-utils/dependencyFixtures'
import { apiClient } from '../api/client'
import DependencyHostsTab from './DependencyHostsTab.vue'
import type { DependencyHostResponse } from '../types/generated'

function listing(hosts: DependencyHostResponse[]): void {
  vi.mocked(apiClient.get).mockResolvedValue({ data: hosts })
}

async function render(isAdmin = true): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(DependencyHostsTab, { props: { isAdmin } })
  await flushPromises()
  return wrapper
}

describe('DependencyHostsTab', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    resetWsHandlers()
  })

  it('renders each dependency as one card that links to its page', async () => {
    listing([
      dependencyHost({
        waiting_count: 2,
        intermittent: true,
        last_check_reachable: false,
        last_checked_at: new Date(Date.now() - 4 * 60_000).toISOString(),
        power: dependencyPower({
          repo_host: { id: 9, ssh_host: 'nas-01' },
          effective_wake_enabled: true,
        }),
      }),
    ])
    const wrapper = await render()

    const card = wrapper.get('a.entity-card')
    expect(card.attributes('href')).toBe('/dependency-hosts/3')
    expect(card.classes()).toContain('entity-card--notable')
    expect(card.get('.card-name').text()).toBe('nas-media')
    const facts = card.get('.cc-facts').text()
    expect(facts).toContain('SMB')
    expect(facts).toContain('nas-media.lan:445')
    expect(facts).toContain('same machine as nas-01')
    const badges = card.findAll('.card-meta .badge').map((b) => b.text())
    expect(badges).toEqual(['2 runs waiting', 'Not always online', 'Wake'])
    const stats = card.findAll('.stat').map((s) => s.text())
    expect(stats).toEqual(['2Schedules', '1Agent defaults', '4m agoLast checked'])
    expect(wrapper.emitted('count')).toEqual([[1]])
  })

  it.each([
    [true, 'badge--success', 'Reachable'],
    [false, 'badge--warning', 'Not answering'],
    [null, 'badge--neutral', 'Not checked yet'],
  ])('reads a last check of %s as %s "%s"', async (reachable, tone, label) => {
    listing([dependencyHost({ last_check_reachable: reachable })])
    const wrapper = await render()
    const badge = wrapper.get('.cc-head-end .badge')
    expect(badge.classes()).toContain(tone)
    expect(badge.text()).toBe(label)
  })

  it('names the protocol from the port, and says when it was never checked', async () => {
    listing([
      dependencyHost({ id: 1, port: 2049 }),
      dependencyHost({ id: 2, port: 22 }),
      dependencyHost({ id: 3, port: 8443 }),
    ])
    const wrapper = await render()
    const protocols = wrapper.findAll('.cc-facts').map((f) => f.find('span').text())
    expect(protocols).toEqual(['NFS', 'SSH', 'TCP'])
    expect(wrapper.text()).toContain('Always online')
    expect(wrapper.text()).toContain('Never')
    expect(wrapper.text()).not.toContain('runs waiting')
    expect(wrapper.text()).not.toContain('same machine as')
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

  it('reloads when something changes, keeping the cards if that fails', async () => {
    listing([dependencyHost()])
    const wrapper = await render()

    listing([dependencyHost(), dependencyHost({ id: 4, name: 'files-01' })])
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(wrapper.findAll('a.entity-card')).toHaveLength(2)

    vi.mocked(apiClient.get).mockRejectedValue(new Error('gone'))
    wsHandlers['DataChanged']!({})
    await flushPromises()
    expect(wrapper.findAll('a.entity-card')).toHaveLength(2)
    expect(wrapper.find('.error-banner').exists()).toBe(false)
  })
})
