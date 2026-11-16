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
  /** `null` when the list could not be read. */
  vms: number | null
  /** `null` when the list could not be read, or the viewer may not read it. */
  databases: number | null
}

/** What the viewer may read. A host's database list names its servers' sockets and users, so only an admin reads it. */
export interface HostSourcesAccess {
  canReadDatabases: () => boolean
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
export function useHostSources(
  hosts: () => readonly AgentRow[],
  access: HostSourcesAccess,
): {
  counts: Ref<Map<number, HostSourceCounts>>
  /** The hosts neither of whose lists could be read: not loading, failed. */
  unreadable: Ref<Set<number>>
  hostsWithout: (kind: HostSourceKind) => AgentRow[]
} {
  const counts = ref(new Map<number, HostSourceCounts>())
  const unreadable = ref(new Set<number>())

  /** Each list is read on its own, so one that fails or is not the viewer's to read never hides the other. */
  async function countOf(read: () => Promise<number>): Promise<number | null> {
    try {
      return await read()
    } catch (e: unknown) {
      logger.error('loading host sources failed', e)
      return null
    }
  }

  async function countFor(host: AgentRow): Promise<HostSourceCounts | null> {
    if (host.is_imported) return { vms: 0, databases: 0 }
    const [vms, databases] = await Promise.all([
      countOf(
        async () =>
          (await getAgentVms(host.hostname, host.domain)).vms.filter((vm) => vm.included).length,
      ),
      access.canReadDatabases()
        ? countOf(
            async () =>
              (await getAgentDatabases(host.hostname, host.domain)).sources.filter(
                (source) => source.included,
              ).length,
          )
        : Promise.resolve(null),
    ])
    return vms === null && databases === null ? null : { vms, databases }
  }

  watch(
    () => hosts().map((h) => h.id),
    async () => {
      const list = hosts()
      const results = await Promise.all(list.map(countFor))
      const next = new Map<number, HostSourceCounts>()
      const failed = new Set<number>()
      list.forEach((host, i) => {
        const result = results[i]
        if (result) next.set(host.id, result)
        else failed.add(host.id)
      })
      counts.value = next
      unreadable.value = failed
    },
    { immediate: true },
  )

  function hostsWithout(kind: HostSourceKind): AgentRow[] {
    return hosts().filter((h) => counts.value.get(h.id)?.[kind] === 0)
  }

  return { counts, unreadable, hostsWithout }
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
    schedules().filter((s) => (kind === 'vms' ? s.vm_snapshot_enabled : s.database_backup_enabled)),
  )
}
