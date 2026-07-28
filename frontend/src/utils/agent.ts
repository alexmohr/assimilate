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
