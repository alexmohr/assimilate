// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { ref } from 'vue'
import { useHostDumpSettings, useSchedulesAskingForDumps } from './useHostDumpSettings'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'

function host(id: number, dumps: boolean): AgentRow {
  return { id, hostname: `host-${id}`, database_backup_enabled: dumps } as unknown as AgentRow
}

describe('useHostDumpSettings', () => {
  it('picks out the hosts with database dumps turned off, in order', () => {
    const { hostsWithDumpsOff } = useHostDumpSettings(() => [
      host(1, false),
      host(2, true),
      host(3, false),
    ])
    expect(hostsWithDumpsOff.value.map((h) => h.id)).toEqual([1, 3])
  })

  it('finds none when every host allows dumps, or there are no hosts', () => {
    expect(useHostDumpSettings(() => [host(1, true)]).hostsWithDumpsOff.value).toEqual([])
    expect(useHostDumpSettings(() => []).hostsWithDumpsOff.value).toEqual([])
  })

  it('follows the hosts as they change', () => {
    const hosts = ref<AgentRow[]>([host(1, true)])
    const { hostsWithDumpsOff } = useHostDumpSettings(() => hosts.value)
    expect(hostsWithDumpsOff.value).toEqual([])

    hosts.value = [host(1, false), host(2, true)]
    expect(hostsWithDumpsOff.value.map((h) => h.id)).toEqual([1])

    hosts.value = [host(1, true), host(2, true)]
    expect(hostsWithDumpsOff.value).toEqual([])
  })
})

function schedule(id: number, dumps: boolean): ScheduleRow {
  return { id, name: `schedule-${id}`, database_backup_enabled: dumps } as unknown as ScheduleRow
}

describe('useSchedulesAskingForDumps', () => {
  it('picks out the schedules that ask for database dumps, in order', () => {
    const { schedulesAskingForDumps } = useSchedulesAskingForDumps(() => [
      schedule(1, true),
      schedule(2, false),
      schedule(3, true),
    ])
    expect(schedulesAskingForDumps.value.map((s) => s.id)).toEqual([1, 3])
  })

  it('follows the schedules as they change', () => {
    const schedules = ref<ScheduleRow[]>([])
    const { schedulesAskingForDumps } = useSchedulesAskingForDumps(() => schedules.value)
    expect(schedulesAskingForDumps.value).toEqual([])

    schedules.value = [schedule(1, false), schedule(2, true)]
    expect(schedulesAskingForDumps.value.map((s) => s.id)).toEqual([2])
  })
})
