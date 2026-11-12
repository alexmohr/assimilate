// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import type { ApiTokenScope } from '../types/generated'

/** What the token table needs of a token. */
export interface ApiTokenRow {
  id: number
  user_id: number
  name: string
  /** Absent from a server that predates token scopes; such a token is full. */
  scope?: ApiTokenScope
  created_at: string
  last_used_at: string | null
  /** When the token stops working; null or absent for one that never expires. */
  expires_at?: string | null
}

/** The expiry offered when creating a token: a preset length, or a date picked. */
export type TokenExpiryChoice = 'never' | '30d' | '90d' | '365d' | 'date'

export const TOKEN_EXPIRY_CHOICES: readonly { value: TokenExpiryChoice; label: string }[] = [
  { value: 'never', label: 'Never' },
  { value: '30d', label: '30 days' },
  { value: '90d', label: '90 days' },
  { value: '365d', label: '1 year' },
  { value: 'date', label: 'On a date' },
]

const EXPIRY_DAYS: Record<Exclude<TokenExpiryChoice, 'never' | 'date'>, number> = {
  '30d': 30,
  '90d': 90,
  '365d': 365,
}

/**
 * The expiry to send for a choice, as an ISO timestamp, or null for none. A
 * picked date expires at the end of that day, local time.
 */
export function tokenExpiresAt(
  choice: TokenExpiryChoice,
  date: string,
  now = new Date(),
): string | null {
  if (choice === 'never') return null
  if (choice === 'date') {
    if (!date) return null
    return new Date(`${date}T23:59:59`).toISOString()
  }
  return new Date(now.getTime() + EXPIRY_DAYS[choice] * 86_400_000).toISOString()
}

/** How a token's expiry reads in the table: past, close, or plain. */
export type TokenExpiryState = 'none' | 'ok' | 'soon' | 'expired'

/** Within this many days an expiry is flagged, so it can be renewed in time. */
const EXPIRY_SOON_DAYS = 14

export function tokenExpiryState(
  expiresAt: string | null | undefined,
  now = Date.now(),
): TokenExpiryState {
  if (!expiresAt) return 'none'
  const at = Date.parse(expiresAt)
  if (at <= now) return 'expired'
  return at - now <= EXPIRY_SOON_DAYS * 86_400_000 ? 'soon' : 'ok'
}

/**
 * What a token's scope is called in the UI. A token listed before the server
 * reported scopes has none, and was a full token.
 */
export function tokenScopeLabel(scope: ApiTokenScope | undefined): string {
  return scope === 'monitoring' ? 'Monitoring (read-only)' : 'Full access'
}

/** Badge tone for a token's scope: a read-only token reads as the safer one. */
export function tokenScopeTone(scope: ApiTokenScope | undefined): BadgeTone {
  return scope === 'monitoring' ? 'info' : 'neutral'
}

/**
 * Splits the allowed-networks text area into entries. Commas, spaces and line
 * breaks all separate entries, the same as the server accepts; validation is
 * left to the server, which names the entry it refused.
 */
export function parseNetworkList(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
}

/** One network per line, the way the text area shows them. */
export function formatNetworkList(networks: readonly string[]): string {
  return networks.join('\n')
}
