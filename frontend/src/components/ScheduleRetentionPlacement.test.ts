// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('../api/repos', () => ({ listRepoStats: vi.fn() }))

import { listRepoStats } from '../api/repos'
import { renderWithPlugins } from '../test-utils'
import { appendOnly, repoFixture } from '../test-utils/repoFixtures'
import ScheduleRetentionPlacement from './ScheduleRetentionPlacement.vue'

async function mount(repoIds: number[]) {
  const wrapper = renderWithPlugins(ScheduleRetentionPlacement, { props: { repoIds } })
  await flushPromises()
  return wrapper
}

describe('ScheduleRetentionPlacement', () => {
  beforeEach(() => {
    vi.mocked(listRepoStats)
      .mockReset()
      .mockResolvedValue([
        repoFixture({ id: 1, name: 'database-hourly' }),
        repoFixture({ id: 2, name: 'nas-offsite', append_only: appendOnly() }),
        repoFixture({ id: 3, name: 'unrelated' }),
      ])
  })

  it('says where retention runs for each target repository', async () => {
    const rows = (await mount([1, 2])).findAll('tbody tr')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.text()).toContain('database-hourly')
    expect(rows[0]!.text()).toContain('After each backup, by the agent')
    expect(rows[1]!.text()).toContain('nas-offsite')
    expect(rows[1]!.find('.badge--violet').exists()).toBe(true)
    expect(rows[1]!.text()).toContain('At repository maintenance, by the server')
  })

  it('leaves out repositories the schedule does not write to', async () => {
    expect((await mount([1])).text()).not.toContain('unrelated')
  })

  it('renders nothing without targets or when the stats cannot be loaded', async () => {
    expect((await mount([])).find('table').exists()).toBe(false)
    vi.mocked(listRepoStats).mockRejectedValue(new Error('boom'))
    expect((await mount([1])).find('table').exists()).toBe(false)
  })
})
