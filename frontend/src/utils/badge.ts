// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { normalizeBackupStatus } from './backupStatus'
import type { RunEventType } from '../types/generated'

/**
 * The tones the shared `.badge` component supports. Defined in
 * `src/style.css`; there is exactly one badge.
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

/** A transient label an agent's header badge shows instead of Online/Offline
    while its host is being reached or powered down around a backup. */
export interface AgentPowerPhase {
  label: string
  tone: BadgeTone
}

/**
 * Maps a live `RunEvent`'s type to the phase `AgentHeader` shows in place of
 * its usual Online/Offline badge, or `null` once the event means the
 * transient phase is over and the badge should reflect connection state
 * again (the agent connected, or the host finished going offline).
 */
export function agentPowerPhase(eventType: RunEventType): AgentPowerPhase | null {
  switch (eventType) {
    case 'reachability_check':
      return { label: 'Checking...', tone: 'neutral' }
    case 'wake_sent':
    case 'host_online':
      return { label: 'Waking host...', tone: 'info' }
    case 'agent_start_sent':
      return { label: 'Starting agent...', tone: 'info' }
    case 'shutdown_sent':
    case 'agent_stop_sent':
      return { label: 'Shutting down...', tone: 'neutral' }
    case 'agent_connected':
    case 'host_offline':
      return null
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
