// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import type { MaintenanceOutcome, MaintenanceRunResponse } from '../types/generated'

// A record keyed by every outcome, so a variant added on the server and missing
// here is a compile error instead of an outcome that quietly fails the narrow.
const MAINTENANCE_OUTCOME_KEYS = {
  ok: true,
  stopped: true,
  check_failed: true,
  failed: true,
} satisfies Record<MaintenanceOutcome, true>

const MAINTENANCE_OUTCOMES: readonly string[] = Object.keys(MAINTENANCE_OUTCOME_KEYS)

/** Narrows the plain string a run row carries to the outcomes the server writes. */
export function isMaintenanceOutcome(value: string): value is MaintenanceOutcome {
  return MAINTENANCE_OUTCOMES.some((outcome) => outcome === value)
}

export interface OutcomeBadge {
  label: string
  tone: BadgeTone
}

function outcomeBadge(outcome: MaintenanceOutcome): OutcomeBadge {
  switch (outcome) {
    case 'ok':
      return { label: 'OK', tone: 'success' }
    case 'stopped':
      return { label: 'Stopped', tone: 'warning' }
    case 'check_failed':
      return { label: 'Check failed', tone: 'danger' }
    case 'failed':
      return { label: 'Failed', tone: 'danger' }
  }
}

/**
 * The badge for a run's outcome. A value this build does not know (a tab left
 * open across a deploy) shows as itself in the neutral tone rather than
 * borrowing the look of a known one.
 */
export function maintenanceOutcomeBadge(
  run: Pick<MaintenanceRunResponse, 'outcome'>,
): OutcomeBadge {
  return isMaintenanceOutcome(run.outcome)
    ? outcomeBadge(run.outcome)
    : { label: run.outcome, tone: 'neutral' }
}

/** Seconds a run took, or null while it is still going. */
export function maintenanceRunSeconds(
  run: Pick<MaintenanceRunResponse, 'started_at' | 'finished_at'>,
): number | null {
  if (run.finished_at === null) return null
  const secs = (Date.parse(run.finished_at) - Date.parse(run.started_at)) / 1000
  return Number.isFinite(secs) && secs >= 0 ? Math.round(secs) : null
}
