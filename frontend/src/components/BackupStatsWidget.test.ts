// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import BackupStatsWidget from './BackupStatsWidget.vue'
import { apiClient } from '../api/client'

// jscpd:ignore-start -- test setup boilerplate (vi.mock factories cannot reference module-scoped helpers)
vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatBytes: (n: number): string => `${n}B`,
  relativeTime: (s: string): string => s,
  formatDuration: (n: number): string => `${n}s`,
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))
// jscpd:ignore-end

const mockGet = vi.mocked(apiClient.get)

function activityEntry(id: number, status: string): Record<string, unknown> {
  return {
    id,
    hostname: `h${id}`,
    target_name: `t${id}`,
    started_at: '',
    finished_at: '',
    status,
    duration_secs: 10,
  }
}

describe('BackupStatsWidget', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('displays the success rate percentage', async () => {
    mockGet.mockResolvedValue({
      data: [activityEntry(1, 'success'), activityEntry(2, 'success'), activityEntry(3, 'failed')],
    })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('67%')
  })

  it('displays failed count', async () => {
    mockGet.mockResolvedValue({
      data: [activityEntry(1, 'success'), activityEntry(2, 'failed'), activityEntry(3, 'failed')],
    })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('2')
  })

  // The range buttons drive the query, so a click has to reach the fetch -
  // a control that only repaints itself looks like it worked and does not.
  it('refetches over the chosen range', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, { props: { repos: [] } })
    await flushPromises()
    expect(mockGet).toHaveBeenCalledWith(expect.stringContaining('days=30'))

    await wrapper
      .findAll('.segmented-option')
      .find((b) => b.text() === '7d')!
      .trigger('click')
    await flushPromises()

    expect(mockGet).toHaveBeenLastCalledWith(expect.stringContaining('days=7'))
  })

  it('shows 0% when no backups have run', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('0%')
  })

  it('navigates to activity, filtered by status, when a mini-stat is clicked', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [] },
    })
    await flushPromises()

    const links = wrapper.findAll('.mini-stat-link')
    await links[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.vm.$route.name).toBe('activity')
    expect(wrapper.vm.$route.query.status).toBeUndefined()

    await links[1]!.trigger('click')
    await flushPromises()
    expect(wrapper.vm.$route.query.status).toBe('success')

    await links[2]!.trigger('click')
    await flushPromises()
    expect(wrapper.vm.$route.query.status).toBe('failed')
  })

  it('refetches stats for the chosen repo', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupStatsWidget, {
      props: { repos: [{ id: 4, name: 'repo-beta' }] },
    })
    await flushPromises()

    await wrapper.find('select').setValue('4')
    await flushPromises()

    expect(mockGet).toHaveBeenLastCalledWith(expect.stringContaining('repo_id=4'))
  })
})
