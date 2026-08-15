// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { normalizeBackupStatus } from './backupStatus'

/**
 * The tones the shared `.badge` component supports. Defined in
 * `src/style.css`; see `docs/contributing/ui-design-audit.md` (F-08) for why
 * there is exactly one badge.
 */
export type BadgeTone = 'success' | 'warning' | 'danger' | 'info' | 'accent' | 'neutral'

export function badgeClass(tone: BadgeTone): string {
  return `badge--${tone}`
}

/** Tone for a backup outcome, shared by every view that renders run status. */
export function backupStatusTone(rawStatus: string): BadgeTone {
  switch (normalizeBackupStatus(rawStatus)) {
    case 'success':
      return 'success'
    case 'warning':
      return 'warning'
    case 'started':
      return 'info'
    case 'pending':
    case 'cancelled':
      return 'neutral'
    case 'failed':
      return 'danger'
  }
}

export function backupStatusBadgeClass(rawStatus: string): string {
  return badgeClass(backupStatusTone(rawStatus))
}

/** Tone for a quota / capacity threshold. */
export function thresholdTone(level: 'ok' | 'warning' | 'critical'): BadgeTone {
  switch (level) {
    case 'ok':
      return 'success'
    case 'warning':
      return 'warning'
    case 'critical':
      return 'danger'
  }
}

/** Tone for a log level. */
export function logLevelTone(level: string): BadgeTone {
  switch (level.toLowerCase()) {
    case 'error':
      return 'danger'
    case 'warn':
    case 'warning':
      return 'warning'
    case 'info':
      return 'info'
    default:
      return 'neutral'
  }
}
