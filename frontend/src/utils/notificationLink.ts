// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  NotificationChannelResponse,
  NotificationLink,
  NotificationLinkTarget,
} from '../types/generated'

export type { NotificationLink, NotificationLinkTarget }

export const DEFAULT_NOTIFICATION_LINK: NotificationLink = { target: 'automatic' }

export interface NotificationLinkOption {
  value: NotificationLinkTarget
  label: string
  hint: string
}

export const NOTIFICATION_LINK_OPTIONS: readonly NotificationLinkOption[] = [
  {
    value: 'automatic',
    label: 'Where the event happened',
    hint: 'A failed or warned backup opens its run in the Activity log; anything else opens its schedule, agent or repository.',
  },
  {
    value: 'activity',
    label: 'Activity log',
    hint: 'The Activity log, filtered to the event.',
  },
  { value: 'agent', label: 'Agent', hint: 'The page of the agent the event is about.' },
  { value: 'schedule', label: 'Schedule', hint: 'The page of the schedule the event is about.' },
  {
    value: 'repository',
    label: 'Repository',
    hint: 'The page of the repository the event is about.',
  },
  { value: 'dashboard', label: 'Dashboard', hint: 'The dashboard, whatever the event.' },
  {
    value: 'custom',
    label: 'Your own address',
    hint: 'A fixed address, such as a status page or a ticket queue.',
  },
  { value: 'none', label: 'No link', hint: 'The notification carries no link.' },
]

/** The channel's link setting. */
export function channelLink(channel: NotificationChannelResponse): NotificationLink {
  return channel.link
}

export function notificationLinkLabel(link: NotificationLink): string {
  if (link.target === 'custom') return link.url ? link.url : 'Your own address'
  return NOTIFICATION_LINK_OPTIONS.find((o) => o.value === link.target)?.label ?? link.target
}

/** What a link setting still needs: an address that starts with http(s):// for `custom`. */
export function notificationLinkProblem(link: NotificationLink): string | null {
  if (link.target !== 'custom') return null
  const url = (link.url ?? '').trim()
  if (!url) return 'an address to link to'
  if (!/^https?:\/\//i.test(url)) return 'an address starting with http:// or https://'
  return null
}
