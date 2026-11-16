// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { getAgentDatabases } from '../api/databases'
import { getAgentVms } from '../api/vms'
import { useHostSources, useSchedulesAsking } from './useHostSources'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'
import type { AgentDatabasesResponse, AgentVmSnapshotResponse } from '../types/generated'

vi.mock('../api/vms', () => ({ getAgentVms: vi.fn() }))
vi.mock('../api/databases', () => ({ getAgentDatabases: vi.fn() }))

const admin = { canReadDatabases: () => true }
const notAdmin = { canReadDatabases: () => false }

function host(id: number, extra: Partial<AgentRow> = {}): AgentRow {
  return {
    id,
    hostname: `host-${id}`,
    domain: null,
    is_imported: false,
    ...extra,
  } as unknown as AgentRow
}

/** What each host has included, by hostname: [included VMs, included database sources]. */
let included: Record<string, [boolean[], boolean[]]> = {}

function flags(hostname: string): [boolean[], boolean[]] {
  const entry = included[hostname]
  if (!entry) throw new Error(`agent ${hostname} is not connected`)
  return entry
}

describe('useHostSources', () => {
  beforeEach(() => {
    included = {}
    vi.mocked(getAgentVms)
      .mockReset()
      .mockImplementation(
        async (hostname) =>
          ({
            vms: flags(hostname)[0].map((inc, i) => ({ name: `vm-${i}`, included: inc })),
          }) as unknown as AgentVmSnapshotResponse,
      )
    vi.mocked(getAgentDatabases)
      .mockReset()
      .mockImplementation(
        async (hostname) =>
          ({
            sources: flags(hostname)[1].map((inc, i) => ({ id: i, included: inc })),
          }) as unknown as AgentDatabasesResponse,
      )
  })

  it('counts only the VMs and database sources each host includes', async () => {
    included = {
      'host-1': [
        [true, false, true],
        [false, true],
      ],
    }
    const { counts } = useHostSources(() => [host(1)], admin)
    await flushPromises()

    expect(counts.value.get(1)).toEqual({ vms: 2, databases: 1 })
  })

  // Was: the hosts with database dumps turned off. A host no longer turns
  // dumps off; it gets none when it includes no database source.
  it('picks out the hosts that include none of a kind, in order', async () => {
    included = {
      'host-1': [[true], []],
      'host-2': [[false], [true]],
      'host-3': [[], [false]],
    }
    const { hostsWithout } = useHostSources(() => [host(1), host(2), host(3)], admin)
    await flushPromises()

    expect(hostsWithout('databases').map((h) => h.id)).toEqual([1, 3])
    expect(hostsWithout('vms').map((h) => h.id)).toEqual([2, 3])
  })

  it('finds none when every host includes some, or there are no hosts', async () => {
    included = { 'host-1': [[true], [true]] }
    const some = useHostSources(() => [host(1)], admin)
    const none = useHostSources(() => [], admin)
    await flushPromises()

    expect(some.hostsWithout('databases')).toEqual([])
    expect(some.hostsWithout('vms')).toEqual([])
    expect(none.hostsWithout('databases')).toEqual([])
  })

  it('passes each host its own domain', async () => {
    included = { 'host-1': [[], []] }
    useHostSources(() => [host(1, { domain: 'corp.example' })], admin)
    await flushPromises()

    expect(getAgentVms).toHaveBeenCalledWith('host-1', 'corp.example')
    expect(getAgentDatabases).toHaveBeenCalledWith('host-1', 'corp.example')
  })

  // Not knowing is not the same as having nothing: a host the server could
  // not ask is not reported as missing sources.
  it('leaves out a host whose lists could not be read', async () => {
    included = { 'host-1': [[], []] }
    const { counts, hostsWithout } = useHostSources(() => [host(1), host(2)], admin)
    await flushPromises()

    expect(counts.value.has(2)).toBe(false)
    expect(hostsWithout('databases').map((h) => h.id)).toEqual([1])
  })

  it('names the hosts whose lists could not be read at all', async () => {
    included = { 'host-1': [[], []] }
    const { unreadable } = useHostSources(() => [host(1), host(2)], admin)
    await flushPromises()

    expect([...unreadable.value]).toEqual([2])
  })

  it('reads only the VMs for a viewer who may not read database lists', async () => {
    included = { 'host-1': [[false], [false]] }
    const { counts, hostsWithout } = useHostSources(() => [host(1)], notAdmin)
    await flushPromises()

    expect(getAgentDatabases).not.toHaveBeenCalled()
    expect(counts.value.get(1)).toEqual({ vms: 0, databases: null })
    expect(hostsWithout('vms').map((h) => h.id)).toEqual([1])
    expect(hostsWithout('databases')).toEqual([])
  })

  // One list failing (say, the database list refused) never hides the other.
  it('keeps the count it could read when the other list fails', async () => {
    included = { 'host-1': [[true], []] }
    vi.mocked(getAgentDatabases).mockRejectedValue(new Error('forbidden'))
    const { counts, hostsWithout } = useHostSources(() => [host(1)], admin)
    await flushPromises()

    expect(counts.value.get(1)).toEqual({ vms: 1, databases: null })
    expect(hostsWithout('databases')).toEqual([])
  })

  // An imported host has no agent to ask, and backs up none of either.
  it('counts an imported host as having none, without asking an agent', async () => {
    const { hostsWithout } = useHostSources(() => [host(1, { is_imported: true })], admin)
    await flushPromises()

    expect(getAgentVms).not.toHaveBeenCalled()
    expect(getAgentDatabases).not.toHaveBeenCalled()
    expect(hostsWithout('vms').map((h) => h.id)).toEqual([1])
    expect(hostsWithout('databases').map((h) => h.id)).toEqual([1])
  })

  // A later host list supersedes one still loading: the late answer for the
  // earlier list never replaces the counts of the hosts now shown.
  it('keeps the counts of the current hosts when an earlier read settles late', async () => {
    included = { 'host-1': [[true], [true]], 'host-2': [[], []] }
    let releaseSlow: () => void = () => {}
    vi.mocked(getAgentVms).mockImplementationOnce(
      (hostname) =>
        new Promise((resolve) => {
          releaseSlow = () =>
            resolve({
              vms: flags(hostname)[0].map((inc, i) => ({ name: `vm-${i}`, included: inc })),
            } as unknown as AgentVmSnapshotResponse)
        }),
    )
    const hosts = ref<AgentRow[]>([host(1)])
    const { counts } = useHostSources(() => hosts.value, admin)

    hosts.value = [host(2)]
    await nextTick()
    await flushPromises()
    expect([...counts.value.keys()]).toEqual([2])

    releaseSlow()
    await flushPromises()
    expect([...counts.value.keys()]).toEqual([2])
  })

  it('follows the hosts as they change', async () => {
    included = {
      'host-1': [[], [true]],
      'host-2': [[], [false]],
    }
    const hosts = ref<AgentRow[]>([host(1)])
    const { hostsWithout } = useHostSources(() => hosts.value, admin)
    await flushPromises()
    expect(hostsWithout('databases')).toEqual([])

    hosts.value = [host(1), host(2)]
    await nextTick()
    await flushPromises()
    expect(hostsWithout('databases').map((h) => h.id)).toEqual([2])

    hosts.value = [host(1)]
    await nextTick()
    await flushPromises()
    expect(hostsWithout('databases')).toEqual([])
  })
})

function schedule(id: number, asks: { vms?: boolean; databases?: boolean } = {}): ScheduleRow {
  return {
    id,
    name: `schedule-${id}`,
    vm_snapshot_enabled: asks.vms ?? false,
    database_backup_enabled: asks.databases ?? false,
  } as unknown as ScheduleRow
}

describe('useSchedulesAsking', () => {
  it('picks out the schedules that take databases, in order', () => {
    const asking = useSchedulesAsking(
      () => [
        schedule(1, { databases: true }),
        schedule(2, { vms: true }),
        schedule(3, { databases: true }),
      ],
      'databases',
    )
    expect(asking.value.map((s) => s.id)).toEqual([1, 3])
  })

  it('picks out the schedules that take virtual machines, in order', () => {
    const asking = useSchedulesAsking(
      () => [
        schedule(1, { vms: true }),
        schedule(2, { databases: true }),
        schedule(3, { vms: true }),
      ],
      'vms',
    )
    expect(asking.value.map((s) => s.id)).toEqual([1, 3])
  })

  it('follows the schedules as they change', () => {
    const schedules = ref<ScheduleRow[]>([])
    const asking = useSchedulesAsking(() => schedules.value, 'databases')
    expect(asking.value).toEqual([])

    schedules.value = [schedule(1), schedule(2, { databases: true })]
    expect(asking.value.map((s) => s.id)).toEqual([2])
  })
})
