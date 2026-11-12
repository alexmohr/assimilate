// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { computed, type ComputedRef } from 'vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'

/**
 * Which of `hosts` have database dumps turned off on the host itself.
 *
 * A backup dumps databases only when both the schedule and its host allow
 * it: the agent combines the two and, when the host says no, writes the run
 * as a warning with no databases in it. Every agent in the list carries its
 * own setting, so a schedule that asks for dumps can say up front which hosts
 * will not give them.
 */
export function useHostDumpSettings(hosts: () => readonly AgentRow[]): {
  hostsWithDumpsOff: ComputedRef<AgentRow[]>
} {
  const hostsWithDumpsOff = computed(() => hosts().filter((h) => !h.database_backup_enabled))
  return { hostsWithDumpsOff }
}

/**
 * Which of `schedules` ask for database dumps: the schedule's half of the
 * same rule. A host with dumps turned off shows these, so the warning on the
 * host and the one on each schedule are worked out the same way.
 */
export function useSchedulesAskingForDumps(schedules: () => readonly ScheduleRow[]): {
  schedulesAskingForDumps: ComputedRef<ScheduleRow[]>
} {
  const schedulesAskingForDumps = computed(() =>
    schedules().filter((s) => s.database_backup_enabled),
  )
  return { schedulesAskingForDumps }
}
