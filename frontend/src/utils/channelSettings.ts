// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { SmtpSecurity } from '../types/generated'

/**
 * The sections of a notification channel's page, in order. Kept out of the
 * component so the view can parse `?section=` into the union before handing
 * it down. Mirrors `dependencyHostSettings.ts`.
 */
export const CHANNEL_SECTIONS = ['settings', 'events', 'scope', 'content', 'history'] as const

export type ChannelSection = (typeof CHANNEL_SECTIONS)[number]

export function isChannelSection(value: unknown): value is ChannelSection {
  return CHANNEL_SECTIONS.some((section) => section === value)
}

export function smtpSecurityLabel(security: SmtpSecurity): string {
  switch (security) {
    case 'starttls':
      return 'STARTTLS'
    case 'tls':
      return 'SSL/TLS'
    case 'none':
      return 'None (insecure)'
  }
}

/**
 * How many recent deliveries a channel's page loads. The server has no
 * per-channel filter, so the page reads this many across every channel and
 * keeps its own.
 */
export const CHANNEL_HISTORY_FETCH = 200
