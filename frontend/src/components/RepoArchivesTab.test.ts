// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import RepoArchivesTab from './RepoArchivesTab.vue'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), delete: vi.fn() },
}))

const ARCHIVES = [
  {
    name: 'web-01-2026-03-01',
    start: '2026-03-01T02:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 300,
    deduplicated_size: 30,
    matched: true,
    agent_hostname: 'web-01',
  },
  {
    name: 'db-01-2026-02-01',
    start: '2026-02-01T02:00:00Z',
    hostname: 'db-01',
    comment: '',
    original_size: 200,
    deduplicated_size: 20,
    matched: false,
    agent_hostname: null,
  },
]

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoArchivesTab, {
    props: {
      repoId: 12,
      repoName: 'server-daily',
      isAdmin: true,
      filterName: null,
      refreshRepo: () => Promise.resolve(),
      ...props,
    },
  })
}

describe('RepoArchivesTab', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get)
      .mockReset()
      .mockResolvedValue({ data: ARCHIVES } as never)
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  // The tab is mounted lazily, only once the user opens it. An earlier
  // version fetched nothing on that first mount, so the panel sat empty
  // until an unrelated DataChanged event arrived - which the whole archive
  // half of the e2e suite caught and no unit test did.
  it('loads the archive list as soon as it is mounted', async () => {
    mount()
    await flushPromises()
    expect(apiClient.get).toHaveBeenCalled()
    expect(vi.mocked(apiClient.get).mock.calls[0][0]).toContain('/repos/12')
  })

  it('renders the loaded archives', async () => {
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.text()).toContain('web-01')
  })

  it('reloads when the view switches to another repository', async () => {
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await wrapper.setProps({ repoId: 99 })
    await flushPromises()

    expect(vi.mocked(apiClient.get).mock.calls[0][0]).toContain('/repos/99')
  })

  it('counts the archives borg could not attribute to an agent', async () => {
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('.unmatched-banner').exists()).toBe(true)
    expect(wrapper.find('.unmatched-banner').text()).toContain('db-01')
  })

  it('hides the unmatched banner when every archive is attributed', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [ARCHIVES[0]] } as never)
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('.unmatched-banner').exists()).toBe(false)
  })

  it('re-scans and reloads on demand', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { matched: 1, remaining_unmatched: 0 },
    } as never)
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await wrapper.find('.unmatched-banner button').trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/repos/12/rescan')
    expect(apiClient.get).toHaveBeenCalled()
  })

  it('asks the view to clear the ?archive= filter rather than touching the route', async () => {
    const wrapper = mount({ filterName: 'web-01-2026-03-01' })
    await flushPromises()

    const banner = wrapper.find('.archive-filter-banner')
    expect(banner.exists()).toBe(true)
    await banner.find('button').trigger('click')
    expect(wrapper.emitted('clear-filter')).toHaveLength(1)
  })
})
