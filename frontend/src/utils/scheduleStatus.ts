// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ScheduleRow } from '../types/schedule'

/**
 * Label for a disabled schedule's status pill, distinguishing why it's off. The
 * scheduler backs off and auto-disables a schedule after repeated failures to reach
 * its target agent - see `docs/agents.md`. Only `auto_disabled_agent_unreachable` and
 * `consecutive_failures` are needed to tell the three cases apart: a human or quota
 * enforcement disabling a schedule always resets `consecutive_failures` to 0, so any
 * positive count on a disabled schedule can only come from the scheduler's own
 * failure-tracking path.
 */
export function scheduleDisabledLabel(
  schedule: Pick<
    ScheduleRow,
    'enabled' | 'auto_disabled_agent_unreachable' | 'consecutive_failures'
  >,
): string {
  if (schedule.enabled) {
    return 'Enabled'
  }
  if (schedule.auto_disabled_agent_unreachable) {
    return 'Auto-disabled · agent unreachable'
  }
  if (schedule.consecutive_failures > 0) {
    return 'Auto-disabled · error'
  }
  return 'Disabled'
}

/**
 * Tooltip for the "Catch-up pending" badge: how many of a schedule's hosts
 * missed a run that will follow when they reconnect. Shared by the schedules
 * list and the repository tab's card so the two never word it differently.
 */
export function catchUpPendingTitle(count: number): string {
  return count === 1
    ? 'One host missed a run while it was offline; it runs when that host reconnects'
    : `${count} hosts missed a run while they were offline; each runs when that host reconnects`
}
