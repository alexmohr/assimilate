// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { normalizeBackupStatus } from './backupStatus'
import type {
  AuditEvent,
  RunEventType,
  SystemEventSeverity,
  TunnelStatus,
} from '../types/generated'

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
    // Never ran: a dependency it needs did not answer, and the run is caught
    // up once it does. Amber like the skip alerts, not red like a failure.
    case 'skipped':
      return 'warning'
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

/**
 * Tone for a system event. The severity comes from the server, which derives
 * it from the event type - the same value decides whether the event can be
 * acknowledged, so the badge and the button can never disagree.
 */
export function systemEventTone(severity: SystemEventSeverity): BadgeTone {
  switch (severity) {
    case 'success':
      return 'success'
    case 'warning':
      return 'warning'
    case 'failed':
      return 'danger'
    case 'info':
      return 'info'
    // Exhaustive over the current union, so this is unreachable at compile
    // time. It guards the deploy-skew case instead: a tab left open while the
    // server gains a new severity would otherwise render an undefined tone.
    default:
      return 'neutral'
  }
}

/**
 * Tone for an audited action: danger for what cannot be undone, warning for
 * what changes a repository, its key, an agent's files or who may do what,
 * accent for what adds something new, info for what only reads or signs a
 * user in or out.
 */
export function auditActionTone(action: AuditEvent['action']): BadgeTone {
  switch (action) {
    case 'delete_archive':
    case 'delete_user':
    case 'delete_group':
    case 'delete_role':
    case 'delete_api_token':
    case 'delete_notification_channel':
    case 'delete_notification_rule':
      return 'danger'
    case 'restore_files':
    case 'key_import':
    case 'key_change_passphrase':
    case 'set_repo_passphrase':
    case 'migrate_encryption':
    case 'reset_password':
    case 'set_user_roles':
    case 'update_group':
    case 'set_group_members':
    case 'update_role':
    case 'set_repo_permission':
    case 'regenerate_agent_token':
    case 'update_notification_channel':
    case 'set_vapid_keys':
      return 'warning'
    case 'create_user':
    case 'create_group':
    case 'create_role':
    case 'create_api_token':
    case 'create_notification_channel':
    case 'create_notification_rule':
      return 'accent'
    case 'download_files':
    case 'key_export':
    case 'login':
    case 'logout':
      return 'info'
    // Exhaustive over the current union; guards a tab left open while the
    // server gains a new action, as in systemEventTone above.
    default:
      return 'neutral'
  }
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
    // Nothing is happening to the host: the schedule asked for a wake the
    // host has no MAC address for, so the badge goes back to reporting
    // connection state.
    case 'wake_unavailable':
      return null
    case 'agent_start_sent':
      return { label: 'Starting agent...', tone: 'info' }
    case 'shutdown_sent':
    case 'agent_stop_sent':
      return { label: 'Shutting down...', tone: 'neutral' }
    // `host_unreachable`: only a dependency records it, and a dependency is
    // not the agent's host.
    case 'agent_connected':
    case 'host_offline':
    case 'agent_stopped':
    case 'host_unreachable':
      return null
  }
}

/**
 * Tone for a reverse SSH tunnel's connection status. The object variant
 * (`{ error: { message } }`) and anything else added to the union later both
 * read as a failure rather than falling through to the connected look.
 */
export function tunnelStatusTone(status: TunnelStatus): BadgeTone {
  if (status === 'connected') return 'success'
  if (status === 'reconnecting') return 'warning'
  if (status === 'disconnected') return 'neutral'
  return 'danger'
}

/** Label for a reverse SSH tunnel's connection status, paired with the tone above. */
export function tunnelStatusLabel(status: TunnelStatus): string {
  if (status === 'connected') return 'Connected'
  if (status === 'disconnected') return 'Disconnected'
  if (status === 'reconnecting') return 'Reconnecting'
  return 'Error'
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
