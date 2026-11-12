// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  ChannelType,
  EventType,
  NotificationChannelResponse,
  NotificationRuleResponse,
} from '../types/generated'

/**
 * The vocabulary the Notifications page and a channel's own page share: every
 * event a channel can be told about, every transport, and how each is spelled
 * in the UI. Both pages used to need it, and two copies drift.
 */
export const EVENT_TYPES: readonly EventType[] = [
  'backup_success',
  'backup_warning',
  'backup_failed',
  'check_success',
  'check_failed',
  'agent_connected',
  'agent_disconnected',
  'schedule_auto_disabled',
  'backup_skipped_agent_offline',
  'backup_skipped_repo_offline',
  'backup_skipped_dependency_offline',
  'backup_waiting_for_host',
  'backup_file_changed',
  'backup_catch_up_abandoned',
]

export const CHANNEL_TYPES: readonly ChannelType[] = ['email', 'webhook', 'web_push']

/** "backup_skipped_agent_offline" reads "Backup skipped agent offline". */
export function eventTypeLabel(et: EventType): string {
  const words = et.split('_')
  return [words[0].charAt(0).toUpperCase() + words[0].slice(1), ...words.slice(1)].join(' ')
}

export function channelTypeLabel(ct: ChannelType): string {
  if (ct === 'email') return 'Email'
  if (ct === 'webhook') return 'Webhook'
  return 'Web push'
}

/** "9 of 14 events", or "No events" for a channel nothing is routed to. */
export function channelEventsLabel(
  rules: readonly NotificationRuleResponse[],
  channelId: number,
): string {
  const count = rules.filter((r) => r.channel_id === channelId).length
  if (count === 0) return 'No events'
  return `${count} of ${EVENT_TYPES.length} events`
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count > 1 ? 's' : ''}`
}

/** "All", or what the channel is narrowed to ("2 repos, 1 host"). */
export function channelScopeLabel(channel: NotificationChannelResponse): string {
  const s = channel.scope
  if (!s) return 'All'
  const parts: string[] = []
  if (s.repo_ids && s.repo_ids.length > 0) parts.push(plural(s.repo_ids.length, 'repo'))
  if (s.agent_ids && s.agent_ids.length > 0) parts.push(plural(s.agent_ids.length, 'host'))
  if (s.schedule_ids && s.schedule_ids.length > 0) {
    parts.push(plural(s.schedule_ids.length, 'schedule'))
  }
  return parts.length > 0 ? parts.join(', ') : 'All'
}
