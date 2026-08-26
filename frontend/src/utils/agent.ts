// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { AgentRow } from '../types/agent'
import { formatDateShort } from './format'

/** Whether a resolved agent is currently known to be offline. */
export function isAgentOffline(agent: AgentRow | null | undefined): boolean {
  return !!agent && agent.is_connected === false
}

/** Human-readable last-seen text for an agent, e.g. "last seen 2 days ago" or "last seen never". */
export function lastSeenText(agent: AgentRow): string {
  const lastSeen = agent.last_seen_at ? formatDateShort(agent.last_seen_at) : 'never'
  return `last seen ${lastSeen}`
}

/**
 * Query params to disambiguate a hostname-keyed `/agents/{hostname}...`
 * request when more than one agent shares that hostname. Omitted entirely
 * when `domain` is unset, so the common case (one agent per hostname) keeps
 * sending the plain request it always has.
 */
export function domainParams(domain: string | null | undefined): { domain?: string } {
  return domain ? { domain } : {}
}
