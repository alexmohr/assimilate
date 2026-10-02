// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ScheduleType } from '../types/schedule'

/**
 * Display name for a schedule's type. The schedule card, the repository's
 * schedules tab, the schedule detail header and the create wizard all spell
 * out what a verify run actually does.
 */
export function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity check'
    case 'verify':
      return 'Verify (extract dry-run)'
  }
}

/**
 * The shorter form the schedules list uses for its type badge and its
 * run-now wording, without the verify parenthetical.
 */
export function scheduleTypeShortLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
    case 'check':
      return scheduleTypeLabel(t)
    case 'verify':
      return 'Verify'
  }
}
