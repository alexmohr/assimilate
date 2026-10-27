// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockFormatBytes } from '../test-utils/sharedMocks'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import type { ServerQuotaResponse } from '../types/generated'
import RepoHostQuotaTile from './RepoHostQuotaTile.vue'
import { pushWs, resetWsMock } from '../test-utils/wsMock'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/format', () => mockFormatBytes())
vi.mock('../utils/logger', () => ({ logger: { error: vi.fn() } }))
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

function serverQuota(overrides: Partial<ServerQuotaResponse> = {}): ServerQuotaResponse {
  return {
    ssh_host: 'nas.lan',
    repo_count: 2,
    total_deduplicated_size: 850,
    configured: true,
    warn_bytes: 800,
    critical_bytes: 1_000,
    warn_action: 'notify_only',
    critical_action: 'block_backups',
    enabled: true,
    updated_at: null,
    ...overrides,
  }
}

async function render(): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(RepoHostQuotaTile, {
    props: { sshHost: 'nas.lan', repoHostId: 5 },
  })
  await flushPromises()
  return wrapper
}

describe('RepoHostQuotaTile', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetWsMock()
  })

  it("shows how full the host is and links to the host's quota section", async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [serverQuota()] })
    const wrapper = await render()
    expect(wrapper.find('.stat-value').text()).toBe('85%')
    expect(wrapper.text()).toContain('850 B of 1000 B on nas.lan')
    expect(wrapper.find('.progress-bar--warning').exists()).toBe(true)
    expect(wrapper.get('a').attributes('href')).toBe('/repo-hosts/5?section=quota')
  })

  it('leaves the bar in its plain colour while the host is under its warning limit', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [serverQuota({ total_deduplicated_size: 500 })],
    })
    const wrapper = await render()
    expect(wrapper.find('.stat-value').text()).toBe('50%')
    const bar = wrapper.get('.progress-bar')
    expect(bar.classes()).not.toContain('progress-bar--warning')
    expect(bar.classes()).not.toContain('progress-bar--danger')
  })

  it('marks a host past its critical limit', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [serverQuota({ total_deduplicated_size: 1_200 })],
    })
    const wrapper = await render()
    expect(wrapper.find('.stat-value').text()).toBe('120%')
    expect(wrapper.find('.progress-bar--danger').exists()).toBe(true)
  })

  it('says no quota is set, with what the host uses', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({
      data: [serverQuota({ configured: false, warn_bytes: null, critical_bytes: null })],
    })
    const wrapper = await render()
    expect(wrapper.find('.stat-value').text()).toBe('Not set')
    expect(wrapper.text()).toContain('850 B used on nas.lan')
    expect(wrapper.find('.progress-track').exists()).toBe(false)
  })

  it('says a switched-off quota is off', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [serverQuota({ enabled: false })] })
    const wrapper = await render()
    expect(wrapper.find('.stat-value').text()).toBe('Off')
  })

  it('follows the host as backups land, and keeps its figures if a reload fails', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [serverQuota()] })
    const wrapper = await render()

    vi.mocked(apiClient.get).mockResolvedValue({
      data: [serverQuota({ total_deduplicated_size: 1_200 })],
    })
    pushWs('DataChanged', undefined)
    await flushPromises()
    expect(wrapper.find('.stat-value').text()).toBe('120%')

    vi.mocked(apiClient.get).mockRejectedValue(new Error('offline'))
    pushWs('DataChanged', undefined)
    await flushPromises()
    expect(wrapper.find('.stat-value').text()).toBe('120%')
  })

  it('renders nothing when the host is not listed or the list fails', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [serverQuota({ ssh_host: 'other' })] })
    expect((await render()).find('.tile').exists()).toBe(false)

    vi.mocked(apiClient.get).mockRejectedValue(new Error('forbidden'))
    expect((await render()).find('.tile').exists()).toBe(false)
  })
})
