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
