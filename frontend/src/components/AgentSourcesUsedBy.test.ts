// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'
import AgentSourcesUsedBy from './AgentSourcesUsedBy.vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleBackupSourcesResponse, ScheduleFilesMode } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn() },
}))

const AGENT = {
  id: 3,
  hostname: 'web-01',
  default_backup_paths: ['/srv', '/etc'],
} as unknown as AgentRow

function schedule(
  id: number,
  overrides: Partial<{
    name: string
    schedule_type: ScheduleRow['schedule_type']
    files_mode: ScheduleFilesMode
    vm_snapshot_enabled: boolean
    database_backup_enabled: boolean
  }> = {},
): ScheduleRow {
  return {
    id,
    name: `Schedule name ${id}`,
    schedule_type: 'backup',
    files_mode: 'host',
    vm_snapshot_enabled: false,
    database_backup_enabled: false,
    ...overrides,
  } as unknown as ScheduleRow
}

function sources(
  overrides: Partial<ScheduleBackupSourcesResponse> = {},
): ScheduleBackupSourcesResponse {
  return {
    backup_sources: [],
    backup_sources_per_agent: [],
    exclude_patterns_per_agent: [],
    include_patterns_per_agent: [],
    commands_per_agent: [],
    file_change_patterns_per_agent: [],
    ...overrides,
  }
}

/** Each schedule's own sources, by schedule id; a missing id fails its request. */
let sourcesById: Record<number, ScheduleBackupSourcesResponse> = {}

async function mount(schedules: ScheduleRow[], agent: AgentRow = AGENT) {
  const wrapper = renderWithPlugins(AgentSourcesUsedBy, { props: { agent, schedules } })
  await flushPromises()
  return wrapper
}

/** Each row's cells, as text. */
function rows(wrapper: Awaited<ReturnType<typeof mount>>): string[][] {
  return wrapper
    .findAll('tbody tr')
    .map((tr) => tr.findAll('td').map((td) => td.text().replace(/\s+/g, ' ').trim()))
}

describe('AgentSourcesUsedBy', () => {
  beforeEach(() => {
    sourcesById = {}
    vi.mocked(apiClient.get)
      .mockReset()
      .mockImplementation(((url: string) => {
        const match = /^\/schedules\/(\d+)\/sources$/.exec(url)
        const found = match ? sourcesById[Number(match[1])] : undefined
        return found ? Promise.resolve({ data: found }) : Promise.reject(new Error('not found'))
      }) as never)
  })

  it('says so when no backup schedule targets the host', async () => {
    const wrapper = await mount([])
    expect(wrapper.text()).toContain('No backup schedule targets this host yet.')
    expect(wrapper.find('table').exists()).toBe(false)
  })

  // Check and verify schedules read a repository, not the host's sources.
  it('leaves out schedules that do not back up', async () => {
    const wrapper = await mount([
      schedule(1, { schedule_type: 'check' }),
      schedule(2, { schedule_type: 'verify' }),
    ])
    expect(apiClient.get).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('No backup schedule targets this host yet.')
  })

  it('lists what each backup schedule takes, fetching paths only for one with its own', async () => {
    sourcesById = { 1: sources(), 2: sources() }
    const wrapper = await mount([
      schedule(1, { name: 'Nightly', vm_snapshot_enabled: true }),
      schedule(2, { name: 'Weekly', database_backup_enabled: true }),
    ])

    // Both take the host's files, so neither has paths of its own to fetch.
    expect(apiClient.get).not.toHaveBeenCalled()
    expect(wrapper.findAll('thead th').map((th) => th.text())).toEqual([
      'Schedule',
      'Files',
      'VMs',
      'Databases',
    ])
    expect(rows(wrapper)).toEqual([
      ['Nightly', 'All 2 paths', 'Yes', 'Off'],
      ['Weekly', 'All 2 paths', 'Off', 'Yes'],
    ])
  })

  it('links each schedule to its sources settings', async () => {
    sourcesById = { 7: sources() }
    const wrapper = await mount([schedule(7, { name: 'Nightly' })])
    expect(wrapper.get('tbody a').attributes('href')).toBe(
      '/schedules/7?tab=settings&section=sources',
    )
  })

  it('names an unnamed schedule by its number', async () => {
    sourcesById = { 9: sources() }
    const wrapper = await mount([schedule(9, { name: '' })])
    expect(wrapper.get('tbody a').text()).toBe('Schedule 9')
  })

  // A schedule's paths for this host win over the paths it sets for every host.
  it('shows the paths a schedule sets for this host', async () => {
    sourcesById = {
      1: sources({
        backup_sources: ['/shared'],
        backup_sources_per_agent: [
          { agent_id: 4, paths: ['/other-host'] },
          { agent_id: 3, paths: ['/srv/www', '/srv/db'] },
        ],
      }),
    }
    const wrapper = await mount([schedule(1, { files_mode: 'own' })])

    expect(rows(wrapper)[0][1]).toBe('/srv/www /srv/db')
    expect(wrapper.find('tbody td code').exists()).toBe(true)
  })

  it('falls back to the paths a schedule sets for every host', async () => {
    sourcesById = {
      1: sources({
        backup_sources: ['/shared'],
        backup_sources_per_agent: [{ agent_id: 4, paths: ['/other-host'] }],
      }),
    }
    const wrapper = await mount([schedule(1, { files_mode: 'own' })])

    expect(rows(wrapper)[0][1]).toBe('/shared')
  })

  it('says Off for a schedule that reads no files, whatever paths it has', async () => {
    sourcesById = { 1: sources({ backup_sources: ['/shared'] }) }
    const wrapper = await mount([schedule(1, { files_mode: 'off' })])

    const cell = wrapper.findAll('tbody td')[1]
    expect(cell.text()).toBe('Off')
    expect(cell.find('code').exists()).toBe(false)
    expect(cell.find('.muted').exists()).toBe(true)
  })

  it('mutes the VM and database cells a schedule leaves out', async () => {
    sourcesById = { 1: sources() }
    const wrapper = await mount([schedule(1, { vm_snapshot_enabled: true })])
    const cells = wrapper.findAll('tbody td')
    expect(cells[2].classes()).not.toContain('muted')
    expect(cells[3].classes()).toContain('muted')
  })

  // One schedule's sources failing to load must not blank the whole table.
  it('still lists a schedule whose sources could not be loaded', async () => {
    const error = vi.spyOn(logger, 'error').mockImplementation(() => {})
    sourcesById = { 2: sources({ backup_sources: ['/shared'] }) }
    const wrapper = await mount([
      schedule(1, { name: 'Broken', files_mode: 'own' }),
      schedule(2, { name: 'Fine', files_mode: 'own' }),
    ])

    // A failure is said as one, never passed off as the host's paths.
    expect(rows(wrapper)).toEqual([
      ['Broken', 'Own paths (could not be loaded)', 'Off', 'Off'],
      ['Fine', '/shared', 'Off', 'Off'],
    ])
    expect(error).toHaveBeenCalledWith('loading schedule sources failed', expect.any(Error))
    error.mockRestore()
  })

  it('says how many paths the host has, or that it has none', async () => {
    const one = await mount([schedule(1, { name: 'Nightly' })], {
      ...AGENT,
      default_backup_paths: ['/etc'],
    })
    expect(rows(one)[0]?.[1]).toBe("The host's 1 path")

    const none = await mount([schedule(1, { name: 'Nightly' })], {
      ...AGENT,
      default_backup_paths: [],
    })
    expect(rows(none)[0]?.[1]).toBe('No paths set on this host')
  })

  it('names a schedule with its own files but no paths', async () => {
    sourcesById = { 1: sources() }
    const wrapper = await mount([schedule(1, { name: 'Empty', files_mode: 'own' })])
    expect(rows(wrapper)[0]?.[1]).toBe('Own files, no paths')
  })

  it('reloads when the schedules change', async () => {
    sourcesById = { 1: sources(), 2: sources() }
    const wrapper = await mount([schedule(1, { name: 'Nightly' })])
    expect(rows(wrapper).map((r) => r[0])).toEqual(['Nightly'])

    await wrapper.setProps({
      schedules: [schedule(1, { name: 'Nightly' }), schedule(2, { name: 'Weekly' })],
    })
    await flushPromises()

    expect(rows(wrapper).map((r) => r[0])).toEqual(['Nightly', 'Weekly'])
  })
})
