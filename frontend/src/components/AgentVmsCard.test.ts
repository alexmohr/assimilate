// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickSectionButton, renderWithPlugins, startEditingSection } from '../test-utils'
import { apiClient } from '../api/client'
import AgentVmsCard from './AgentVmsCard.vue'
import type { AgentRow } from '../types/agent'
import type { AgentVmResponse, AgentVmSnapshotResponse } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), put: vi.fn(), post: vi.fn() },
}))

const GIB = 1024 * 1024 * 1024

const AGENT = { hostname: 'virt-host-01', domain: null } as unknown as AgentRow

function vm(overrides: Partial<AgentVmResponse> = {}): AgentVmResponse {
  return {
    name: 'web01',
    included: true,
    limit_bytes: null,
    effective_limit_bytes: 200 * GIB,
    state: 'running',
    mode: 'incremental',
    disk_count: 1,
    disk_bytes: 40 * GIB,
    staged_bytes: 42 * GIB,
    chain_length: 4,
    last_error: null,
    last_scanned_at: '2026-09-03T02:00:00Z',
    last_staged_at: '2026-09-03T02:04:00Z',
    ...overrides,
  }
}

function response(vms: AgentVmResponse[] = [vm()]): AgentVmSnapshotResponse {
  return {
    settings: {
      enabled: true,
      staging_dir: '/srv/vm-staging',
      full_interval: 7,
      timeout_seconds: 1800,
      default_limit_bytes: 200 * GIB,
    },
    vms,
  }
}

async function mount(props: Record<string, unknown> = {}) {
  const wrapper = renderWithPlugins(AgentVmsCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
  await flushPromises()
  return wrapper
}

describe('AgentVmsCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.get).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.put).mockResolvedValue({ data: response() } as never)
    vi.mocked(apiClient.post).mockResolvedValue({ data: response() } as never)
  })

  it('summarizes the host settings it loaded', async () => {
    const text = (await mount()).text()
    expect(text).toContain('/srv/vm-staging')
    expect(text).toContain('7 increments')
    expect(text).toContain('1800 seconds per domain')
    expect(text).toContain('200 GiB')
  })

  it('lists each domain with its state, mode and staged size', async () => {
    const text = (await mount()).text()
    expect(text).toContain('web01')
    expect(text).toContain('Running')
    expect(text).toContain('Incremental')
    expect(text).toContain('full + 4 increments')
    expect(text).toContain('42.0 GB of 200.0 GB')
  })

  it('says a domain inherits the host default until it is overridden', async () => {
    const wrapper = await mount()
    expect(wrapper.text()).toContain('Host default')

    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ limit_bytes: 500 * GIB, effective_limit_bytes: 500 * GIB })]),
    } as never)
    const overridden = await mount()
    expect(overridden.text()).toContain('Overridden')
  })

  it('shows the reason a domain failed its last run', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ last_error: 'a full backup needs about 21.0 GiB' })]),
    } as never)
    expect((await mount()).text()).toContain('a full backup needs about 21.0 GiB')
  })

  it('offers a rescan when the host has never been scanned', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: response([]) } as never)
    const wrapper = await mount()
    expect(wrapper.text()).toContain('No domains reported')
  })

  it('asks the agent to rescan its host', async () => {
    const wrapper = await mount()
    const rescan = wrapper.findAll('button').find((b) => b.text().includes('Rescan host'))
    await rescan?.trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/virt-host-01/vms/scan', {}, { params: {} })
  })

  it('reports a scan the agent could not run', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('agent is not connected'))
    const wrapper = await mount()
    const rescan = wrapper.findAll('button').find((b) => b.text().includes('Rescan host'))
    await rescan?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('agent is not connected')
  })

  it('saves the host settings in bytes, from the GiB the operator typed', async () => {
    const wrapper = await mount()
    await startEditingSection(wrapper)

    await wrapper.find<HTMLInputElement>('#vm-staging-dir').setValue('/data/vm-staging')
    await wrapper.find<HTMLInputElement>('#vm-full-interval').setValue('14')
    await wrapper.find<HTMLInputElement>('#vm-default-limit').setValue('500')
    await clickSectionButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vm-snapshot',
      {
        enabled: true,
        staging_dir: '/data/vm-staging',
        full_interval: 14,
        timeout_seconds: 1800,
        default_limit_bytes: 500 * GIB,
      },
      { params: {} },
    )
  })

  it('saves a per-domain limit as soon as it is entered', async () => {
    const wrapper = await mount()
    const limit = wrapper.find<HTMLInputElement>('input.vm-limit')
    limit.element.value = '50'
    await limit.trigger('change')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vms/web01',
      { included: true, limit_bytes: 50 * GIB },
      { params: {} },
    )
  })

  it('clears a per-domain limit back to the host default', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: response([vm({ limit_bytes: 500 * GIB, effective_limit_bytes: 500 * GIB })]),
    } as never)
    const wrapper = await mount()
    const limit = wrapper.find<HTMLInputElement>('input.vm-limit')
    limit.element.value = ''
    await limit.trigger('change')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/virt-host-01/vms/web01',
      { included: true, limit_bytes: null },
      { params: {} },
    )
  })

  it('leaves an imported host read only', async () => {
    const wrapper = await mount({ canEdit: false })
    expect(wrapper.text()).not.toContain('Rescan host')
    expect(wrapper.find<HTMLInputElement>('input.vm-limit').element.disabled).toBe(true)
  })
})
