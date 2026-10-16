// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { apiClient } from '../api/client'
import AgentDatabasesCard from './AgentDatabasesCard.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'
import type { AgentDatabaseSourceResponse, AgentDatabasesResponse } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), put: vi.fn(), post: vi.fn() },
}))

const AGENT = { hostname: 'k3s-node-01', domain: null } as unknown as AgentRow

function source(overrides: Partial<AgentDatabaseSourceResponse> = {}): AgentDatabaseSourceResponse {
  return {
    namespace: 'shop',
    name: 'orders-db',
    included: true,
    kind: 'cnpg_cluster',
    engine: 'postgresql',
    pod: 'orders-db-2',
    container: 'postgres',
    databases: ['orders', 'payments'],
    last_error: null,
    last_scanned_at: '2026-10-14T02:00:00Z',
    ...overrides,
  }
}

function response(
  sources: AgentDatabaseSourceResponse[] = [source()],
  overrides: Partial<AgentDatabasesResponse['settings']> = {},
): AgentDatabasesResponse {
  return {
    settings: {
      enabled: true,
      dump_dir: '/var/lib/assimilate/database-dumps',
      kubeconfig_path: '/etc/assimilate/kubeconfig',
      selection: 'all',
      ...overrides,
    },
    sources,
  }
}

async function mount(props: Record<string, unknown> = {}) {
  const wrapper = renderWithPlugins(AgentDatabasesCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
  await flushPromises()
  return wrapper
}

describe('AgentDatabasesCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.put).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.post).mockResolvedValue({ data: response() } as never)
  })

  it('summarizes the host settings and lists each source with its databases', async () => {
    const text = (await mount()).text()
    expect(text).toContain('Allowed')
    expect(text).toContain('/etc/assimilate/kubeconfig')
    expect(text).toContain('/var/lib/assimilate/database-dumps')
    expect(text).toContain('shop/orders-db')
    expect(text).toContain('orders-db-2 (postgres)')
    expect(text).toContain('CloudNativePG')
    expect(text).toContain('orders, payments')
    expect(apiClient.get).toHaveBeenCalledWith('/agents/k3s-node-01/databases', expect.anything())
  })

  it('says when no kubeconfig is set and when a source was never scanned', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response(
        [
          source({
            kind: null,
            engine: null,
            pod: null,
            container: null,
            databases: [],
            last_scanned_at: null,
            last_error: 'not found in the last scan',
          }),
        ],
        { kubeconfig_path: null, selection: 'selected' },
      ),
    } as never)
    const text = (await mount()).text()
    expect(text).toContain('Agent default')
    expect(text).toContain('Only the sources selected below')
    expect(text).toContain('Not scanned')
    expect(text).toContain('None found')
    expect(text).toContain('Never')
    expect(text).toContain('not found in the last scan')
    expect(text).toContain('Turn a source on to dump its databases')
  })

  it('shows an empty state before the first scan', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: response([]) } as never)
    expect((await mount()).text()).toContain('No database sources found')
  })

  it('reports a failed load', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    expect((await mount()).text()).toContain('boom')
  })

  it('saves the settings, sending an empty kubeconfig as null', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find('#db-kubeconfig').setValue('   ')
    await wrapper.find('#db-dump-dir').setValue(' /srv/dumps ')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      {
        enabled: true,
        selection: 'all',
        dump_dir: '/srv/dumps',
        kubeconfig_path: null,
      },
      expect.anything(),
    )
  })

  it('keeps the form open and says why when saving fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('the dump directory must not be below'))
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')
    expect(wrapper.text()).toContain('the dump directory must not be below')
    expect(wrapper.find('#db-dump-dir').exists()).toBe(true)
  })

  it('throws away an abandoned edit', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find('#db-dump-dir').setValue('/elsewhere')
    await clickSectionButton(wrapper, 'Cancel')
    expect(wrapper.text()).toContain('/var/lib/assimilate/database-dumps')
    expect(wrapper.text()).not.toContain('/elsewhere')
  })

  it('scans with the saved kubeconfig, or the one in the open form', async () => {
    const wrapper = await mount()
    await clickSectionButton(wrapper, 'Scan cluster')
    expect(apiClient.post).toHaveBeenLastCalledWith(
      '/agents/k3s-node-01/databases/scan',
      { kubeconfig_path: null },
      expect.anything(),
    )

    await startEditingSection(wrapper)
    await wrapper.find('#db-kubeconfig').setValue('/tmp/try-this')
    await clickSectionButton(wrapper, 'Scan cluster')
    expect(apiClient.post).toHaveBeenLastCalledWith(
      '/agents/k3s-node-01/databases/scan',
      { kubeconfig_path: '/tmp/try-this' },
      expect.anything(),
    )
    // The form keeps what was typed.
    expect((wrapper.find('#db-kubeconfig').element as HTMLInputElement).value).toBe('/tmp/try-this')
  })

  it('reports a failed scan', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('could not reach the cluster'))
    const wrapper = await mount()
    await clickSectionButton(wrapper, 'Scan cluster')
    expect(wrapper.text()).toContain('could not reach the cluster')
  })

  it('turns a source off, and says when that fails', async () => {
    const wrapper = await mount()
    wrapper.findComponent(ToggleSwitch).vm.$emit('update:modelValue', false)
    await flushPromises()
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/databases/shop/orders-db',
      { included: false },
      expect.anything(),
    )

    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    wrapper.findComponent(ToggleSwitch).vm.$emit('update:modelValue', true)
    await flushPromises()
    expect(wrapper.text()).toContain('agent offline')
  })

  it('saves a change to the permission and the selection mode', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    wrapper.findComponent(ToggleSwitch).vm.$emit('update:modelValue', false)
    wrapper.findComponent({ name: 'BaseSegmented' }).vm.$emit('update:modelValue', 'selected')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/k3s-node-01/database-backup',
      expect.objectContaining({ enabled: false, selection: 'selected' }),
      expect.anything(),
    )
  })

  it.each([
    ['allow database dumps', 'Each schedule decides whether its own runs include them'],
    ['how the agent reaches the cluster', 'running commands in a database pod is a privileged'],
    ['where the dumps are streamed from', 'holds one named pipe per database'],
  ])('explains %s when asked', async (label, text) => {
    const wrapper = await mount()
    await startEditingSection(wrapper)
    await wrapper.find(`[aria-label="Help: ${label}"]`).trigger('click')
    expect(wrapper.find('.help-hint-pop').text()).toContain(text)
  })

  it('offers no scan for an imported host', async () => {
    const wrapper = await mount({ canEdit: false })
    expect(wrapper.text()).not.toContain('Scan cluster')
  })
})
