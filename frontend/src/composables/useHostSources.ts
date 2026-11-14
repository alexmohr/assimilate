// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'
import { getAgentDatabases } from '../api/databases'
import { getAgentVms } from '../api/vms'
import { logger } from '../utils/logger'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'

/** The two kinds of source a host sets up beyond files. */
export type HostSourceKind = 'vms' | 'databases'

/** How many of each a host backs up: the domains and database sources it includes. */
export interface HostSourceCounts {
  vms: number
  databases: number
}

/**
 * What each of `hosts` has set up to back up beyond files.
 *
 * A host no longer turns VM or database backups on or off: it backs up
 * whatever it includes, and each schedule takes that or opts out. So "this
 * schedule asks for databases but gets none" now means the host includes no
 * database source (none found, or none selected), which only the host's own
 * VM and database lists say. A host whose lists could not be read is left
 * out of `counts` rather than reported as having nothing.
 */
export function useHostSources(hosts: () => readonly AgentRow[]): {
  counts: Ref<Map<number, HostSourceCounts>>
  hostsWithout: (kind: HostSourceKind) => AgentRow[]
} {
  const counts = ref(new Map<number, HostSourceCounts>())

  async function countFor(host: AgentRow): Promise<HostSourceCounts | null> {
    if (host.is_imported) return { vms: 0, databases: 0 }
    try {
      const [vms, databases] = await Promise.all([
        getAgentVms(host.hostname, host.domain),
        getAgentDatabases(host.hostname, host.domain),
      ])
      return {
        vms: vms.vms.filter((vm) => vm.included).length,
        databases: databases.sources.filter((source) => source.included).length,
      }
    } catch (e: unknown) {
      logger.error('loading host sources failed', e)
      return null
    }
  }

  watch(
    () => hosts().map((h) => h.id),
    async () => {
      const list = hosts()
      const results = await Promise.all(list.map(countFor))
      const next = new Map<number, HostSourceCounts>()
      list.forEach((host, i) => {
        const result = results[i]
        if (result) next.set(host.id, result)
      })
      counts.value = next
    },
    { immediate: true },
  )

  function hostsWithout(kind: HostSourceKind): AgentRow[] {
    return hosts().filter((h) => counts.value.get(h.id)?.[kind] === 0)
  }

  return { counts, hostsWithout }
}

/**
 * Which of `schedules` take this kind of source: the schedule's half of the
 * same rule. A host with none set up lists these, so the warning on the host
 * and the one on each schedule are worked out the same way.
 */
export function useSchedulesAsking(
  schedules: () => readonly ScheduleRow[],
  kind: HostSourceKind,
): ComputedRef<ScheduleRow[]> {
  return computed(() =>
    schedules().filter((s) =>
      kind === 'vms' ? s.vm_snapshot_enabled : s.database_backup_enabled,
    ),
  )
}
