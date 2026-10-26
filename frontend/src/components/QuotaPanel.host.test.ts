// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockErrorUtils, mockFormatBytes } from '../test-utils/sharedMocks'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import type { ServerQuotaResponse } from '../types/generated'
import QuotaPanel from './QuotaPanel.vue'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/format', () => mockFormatBytes())
vi.mock('../utils/error', () => mockErrorUtils())

// The host's quota: the same panel as a repository's, given `sshHost` instead
// of `repoId`, read from and written to the server-quota endpoints.

function serverQuota(overrides: Partial<ServerQuotaResponse> = {}): ServerQuotaResponse {
  return {
    ssh_host: 'nas.lan',
    repo_count: 2,
    total_deduplicated_size: 9_000,
    configured: true,
    warn_bytes: 8_000,
    critical_bytes: 10_000,
    warn_action: 'notify_only',
    critical_action: 'block_backups',
    enabled: true,
    updated_at: null,
    ...overrides,
  }
}

function setup(quotas: ServerQuotaResponse[]): void {
  vi.mocked(apiClient.get).mockResolvedValue({ data: quotas })
  vi.mocked(apiClient.put).mockResolvedValue({ data: quotas[0] })
  vi.mocked(apiClient.delete).mockResolvedValue({})
}

async function render(isAdmin = true): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(QuotaPanel, { props: { sshHost: 'nas.lan', isAdmin } })
  await flushPromises()
  return wrapper
}

function button(wrapper: ReturnType<typeof renderWithPlugins>, label: string): HTMLButtonElement {
  const found = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!found) throw new Error(`no ${label} button`)
  return found.element as HTMLButtonElement
}

describe('QuotaPanel for a repository host', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it("measures the host's combined size against its own quota, ignoring other hosts", async () => {
    setup([
      serverQuota({ ssh_host: 'other.lan', total_deduplicated_size: 1, critical_bytes: 2 }),
      serverQuota(),
    ])
    const wrapper = await render()
    expect(apiClient.get).toHaveBeenCalledWith('/server-quotas')
    expect(wrapper.text()).toContain('9000 B used')
    expect(wrapper.text()).toContain('10000 B limit')
    expect(wrapper.text()).toContain('Block backups')
    expect(wrapper.find('.badge-warn').exists()).toBe(true)
  })

  it('says nothing is set on a host without a quota, with its usage', async () => {
    setup([serverQuota({ configured: false, warn_bytes: null, critical_bytes: null })])
    const wrapper = await render()
    expect(wrapper.text()).toContain('No quota configured for the repositories on this host.')
    expect(wrapper.text()).toContain('9000 B used in total.')
    expect(wrapper.text()).toContain('Configure quota')
  })

  it("treats a host the list doesn't name as having no quota", async () => {
    setup([])
    const wrapper = await render()
    expect(wrapper.text()).toContain('No quota configured for the repositories on this host.')
  })

  it("saves the form to the host's server quota", async () => {
    setup([serverQuota()])
    const wrapper = await render()
    button(wrapper, 'Edit').click()
    await flushPromises()
    await wrapper.find('select[aria-label="Warning action"]').setValue('disable_schedule')
    button(wrapper, 'Save').click()
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith('/server-quotas/nas.lan', {
      warn_bytes: expect.any(Number),
      critical_bytes: expect.any(Number),
      warn_action: 'disable_schedule',
      critical_action: 'block_backups',
      enabled: true,
    })
  })

  it('removes the quota and shows the host without one', async () => {
    setup([serverQuota()])
    const wrapper = await render()
    vi.mocked(apiClient.get).mockResolvedValue({ data: [serverQuota({ configured: false })] })
    button(wrapper, 'Remove').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/server-quotas/nas.lan')
    expect(wrapper.text()).toContain('No quota configured for the repositories on this host.')
  })

  it('reports a removal that failed', async () => {
    setup([serverQuota()])
    const wrapper = await render()
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('refused'))
    button(wrapper, 'Remove').click()
    await flushPromises()
    expect(wrapper.find('.state-error').text()).toBe('API error')
  })

  it('offers no Edit or Remove to a non-admin', async () => {
    setup([serverQuota()])
    const wrapper = await render(false)
    const labels = wrapper.findAll('button').map((b) => b.text().trim())
    expect(labels).not.toContain('Edit')
    expect(labels).not.toContain('Remove')
  })
})
