// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { relativeTime } from './format'
import type {
  DeliveryStatus,
  NotificationChannelResponse,
  NotificationDeliveryResponse,
} from '../types/generated'

/** The tone a delivery's outcome is drawn in: a status dot and its text. */
export type DeliveryTone = 'success' | 'danger' | 'warning'

/** A channel that failed at least once in the recently loaded history. */
export interface ChannelFailureSummary {
  channel: NotificationChannelResponse
  /** Failed attempts among the channel's recent deliveries. */
  failed: number
  /** All of the channel's recent deliveries, failed or not. */
  total: number
  /** The newest failure's message, when the server recorded one. */
  lastError: string | null
}

/** How long a short error may run before it is cut off with an ellipsis. */
const SHORT_ERROR_LENGTH = 60

/** Newest first, whatever order the history arrived in. */
export function newestFirst(
  deliveries: readonly NotificationDeliveryResponse[],
): NotificationDeliveryResponse[] {
  return [...deliveries].sort(
    (a, b) => new Date(b.attempted_at).getTime() - new Date(a.attempted_at).getTime(),
  )
}

/** The `count` newest deliveries, for the "Recent deliveries" panel. */
export function recentDeliveries(
  deliveries: readonly NotificationDeliveryResponse[],
  count: number,
): NotificationDeliveryResponse[] {
  return newestFirst(deliveries).slice(0, count)
}

/** Each channel's newest delivery, keyed by channel id. */
export function latestDeliveryByChannel(
  deliveries: readonly NotificationDeliveryResponse[],
): Map<number, NotificationDeliveryResponse> {
  const latest = new Map<number, NotificationDeliveryResponse>()
  for (const d of newestFirst(deliveries)) {
    if (!latest.has(d.channel_id)) latest.set(d.channel_id, d)
  }
  return latest
}

/**
 * Every channel with a failed delivery in the loaded history, in channel
 * order: how many of its recent deliveries failed, out of how many, and the
 * newest failure's message. A channel with no failures is left out, so an
 * empty result means there is nothing to warn about.
 */
export function channelFailureSummaries(
  channels: readonly NotificationChannelResponse[],
  deliveries: readonly NotificationDeliveryResponse[],
): ChannelFailureSummary[] {
  const ordered = newestFirst(deliveries)
  const out: ChannelFailureSummary[] = []
  for (const channel of channels) {
    const own = ordered.filter((d) => d.channel_id === channel.id)
    const failures = own.filter((d) => d.status === 'failed')
    if (failures.length === 0) continue
    out.push({
      channel,
      failed: failures.length,
      total: own.length,
      lastError: failures[0].error_message,
    })
  }
  return out
}

/**
 * Channels with a failure first, the rest in their original order: the one
 * that needs attention is the one a reader should not have to look for.
 */
export function sortChannelsByTrouble(
  channels: readonly NotificationChannelResponse[],
  failing: ReadonlySet<number>,
): NotificationChannelResponse[] {
  return [
    ...channels.filter((c) => failing.has(c.id)),
    ...channels.filter((c) => !failing.has(c.id)),
  ]
}

/**
 * Only the host of a webhook URL. The rest of it - the path, the query, any
 * user info - can carry a token, so it never reaches the list.
 */
export function webhookHost(url: string): string {
  try {
    return new URL(url).hostname || 'Webhook'
  } catch {
    return 'Invalid URL'
  }
}

/** Where a channel delivers, without anything secret in it. */
export function channelTarget(channel: NotificationChannelResponse): string {
  if (channel.channel_type === 'webhook') return webhookHost(channel.config?.url ?? '')
  if (channel.channel_type === 'email') {
    const recipients = channel.config?.to_addresses ?? []
    if (recipients.length === 0) return 'No recipients'
    return recipients.length > 1 ? `${recipients[0]} +${recipients.length - 1} more` : recipients[0]
  }
  return 'Subscribed browsers'
}

export function deliveryTone(status: DeliveryStatus): DeliveryTone {
  if (status === 'sent') return 'success'
  if (status === 'failed') return 'danger'
  return 'warning'
}

/** An error message cut to one short line. */
export function shortError(message: string | null): string {
  const line = (message ?? '').split('\n')[0].trim()
  if (line.length <= SHORT_ERROR_LENGTH) return line
  return `${line.slice(0, SHORT_ERROR_LENGTH - 3).trimEnd()}...`
}

/** "Sent 1d ago", "Failed 2h ago · timeout", or "Nothing sent yet". */
export function lastDeliveryLabel(delivery: NotificationDeliveryResponse | undefined): string {
  if (!delivery) return 'Nothing sent yet'
  const when = relativeTime(delivery.attempted_at)
  if (delivery.status === 'sent') return `Sent ${when}`
  if (delivery.status === 'pending') return `Pending ${when}`
  const error = shortError(delivery.error_message)
  return error ? `Failed ${when} · ${error}` : `Failed ${when}`
}
